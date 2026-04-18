#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TARGET_DIR="$ROOT/target"
SESSION_PREFIX="${SESSION_PREFIX:-fpy-startup-bench}"
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
FPY_BIN="${FPY_BIN:-$ROOT/target/release/fpy}"
FPY_CMD="${FPY_CMD:-$FPY_BIN run --python $PYTHON_BIN}"
if [ -z "${IPYTHON_CMD+x}" ]; then
  if [ -x "$ROOT/.venv/bin/ipython" ]; then
    IPYTHON_CMD="$ROOT/.venv/bin/ipython"
  else
    IPYTHON_CMD="ipython"
  fi
fi
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

pane_has_usable_input() {
  bench_name=$1
  pane_text=$2
  case "$bench_name" in
    fpy)
      printf '%s' "$pane_text" | grep -F "Ctrl-P palette" >/dev/null 2>&1
      return $?
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

pane_is_submit_ready() {
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
      pane_has_usable_input "$bench_name" "$pane_text"
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

  input_ready_ns=
  input_ready_streak=0
  while :; do
    pane_text=$(capture_pane "$session")
    if pane_has_usable_input "$bench_name" "$pane_text"; then
      input_ready_streak=$((input_ready_streak + 1))
      if [ "$input_ready_streak" -ge 2 ]; then
        input_ready_ns=$(now_ns)
        break
      fi
    else
      input_ready_streak=0
    fi
    if [ "$(now_ns)" -ge "$timeout_deadline" ]; then
      printf '%s\n' "$pane_text" > "$log_prefix-timeout-input-ready.log"
      cleanup_session "$session"
      printf 'timed out waiting for %s usable input; pane saved to %s-timeout-input-ready.log\n' \
        "$bench_name" "$log_prefix" >&2
      exit 1
    fi
    sleep "$POLL_SECONDS"
  done

  tmux send-keys -t "$session" -l "1+1"

  submit_ready_ns=
  submit_ready_streak=0
  while :; do
    pane_text=$(capture_pane "$session")
    if pane_is_submit_ready "$bench_name" "$pane_text"; then
      submit_ready_streak=$((submit_ready_streak + 1))
      if [ "$submit_ready_streak" -ge 2 ]; then
        submit_ready_ns=$(now_ns)
        break
      fi
    else
      submit_ready_streak=0
    fi
    if [ "$(now_ns)" -ge "$timeout_deadline" ]; then
      printf '%s\n' "$pane_text" > "$log_prefix-timeout-submit-ready.log"
      cleanup_session "$session"
      printf 'timed out waiting for %s submit readiness; pane saved to %s-timeout-submit-ready.log\n' \
        "$bench_name" "$log_prefix" >&2
      exit 1
    fi
    sleep "$POLL_SECONDS"
  done

  tmux send-keys -t "$session" Enter

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

  input_ready_ms=$(( (input_ready_ns - start_ns) / 1000000 ))
  submit_ready_ms=$(( (submit_ready_ns - start_ns) / 1000000 ))
  first_result_ms=$(( (result_ns - start_ns) / 1000000 ))
  submit_to_result_ms=$(( (result_ns - submit_ready_ns) / 1000000 ))
  printf '%s %s %s %s %s\n' \
    "$sample_index" \
    "$input_ready_ms" \
    "$submit_ready_ms" \
    "$first_result_ms" \
    "$submit_to_result_ms"
}

summarize_samples() {
  bench_name=$1
  sample_file=$2
  awk -v name="$bench_name" '
    BEGIN {
      input_sum = 0;
      submit_sum = 0;
      result_sum = 0;
      submit_to_result_sum = 0;
      input_min = -1;
      input_max = 0;
      submit_min = -1;
      submit_max = 0;
      result_min = -1;
      result_max = 0;
      submit_to_result_min = -1;
      submit_to_result_max = 0;
      count = 0;
    }
    {
      count += 1;
      input = $2;
      submit = $3;
      result = $4;
      submit_to_result = $5;
      input_sum += input;
      submit_sum += submit;
      result_sum += result;
      submit_to_result_sum += submit_to_result;
      if (input_min == -1 || input < input_min) input_min = input;
      if (input > input_max) input_max = input;
      if (submit_min == -1 || submit < submit_min) submit_min = submit;
      if (submit > submit_max) submit_max = submit;
      if (result_min == -1 || result < result_min) result_min = result;
      if (result > result_max) result_max = result;
      if (submit_to_result_min == -1 || submit_to_result < submit_to_result_min) submit_to_result_min = submit_to_result;
      if (submit_to_result > submit_to_result_max) submit_to_result_max = submit_to_result;
    }
    END {
      printf "%s %d %d %d %d %d %d %d %d %d %d %d\n",
        name,
        count,
        input_sum / count,
        input_min,
        input_max,
        submit_sum / count,
        submit_min,
        submit_max,
        result_sum / count,
        result_min,
        result_max,
        submit_to_result_sum / count;
    }
  ' "$sample_file"
}

print_summary() {
  fpy_summary=$1
  ipython_summary=$2

  set -- $fpy_summary
  fpy_input_avg=$3
  fpy_input_min=$4
  fpy_input_max=$5
  fpy_submit_avg=$6
  fpy_submit_min=$7
  fpy_submit_max=$8
  fpy_result_avg=$9
  fpy_result_min=${10}
  fpy_result_max=${11}
  fpy_submit_to_result_avg=${12}

  set -- $ipython_summary
  ipython_input_avg=$3
  ipython_input_min=$4
  ipython_input_max=$5
  ipython_submit_avg=$6
  ipython_submit_min=$7
  ipython_submit_max=$8
  ipython_result_avg=$9
  ipython_result_min=${10}
  ipython_result_max=${11}
  ipython_submit_to_result_avg=${12}

  input_ratio=$(awk -v a="$fpy_input_avg" -v b="$ipython_input_avg" 'BEGIN { printf "%.2f", a / b }')
  submit_ratio=$(awk -v a="$fpy_submit_avg" -v b="$ipython_submit_avg" 'BEGIN { printf "%.2f", a / b }')
  result_ratio=$(awk -v a="$fpy_result_avg" -v b="$ipython_result_avg" 'BEGIN { printf "%.2f", a / b }')

  printf '\nSummary\n'
  printf 'target   input_avg_ms input_min_ms input_max_ms submit_avg_ms submit_min_ms submit_max_ms first_result_avg_ms first_result_min_ms first_result_max_ms submit_to_result_avg_ms\n'
  printf 'fpy      %12s %12s %12s %13s %13s %13s %19s %19s %19s %23s\n' \
    "$fpy_input_avg" "$fpy_input_min" "$fpy_input_max" \
    "$fpy_submit_avg" "$fpy_submit_min" "$fpy_submit_max" \
    "$fpy_result_avg" "$fpy_result_min" "$fpy_result_max" \
    "$fpy_submit_to_result_avg"
  printf 'ipython  %12s %12s %12s %13s %13s %13s %19s %19s %19s %23s\n' \
    "$ipython_input_avg" "$ipython_input_min" "$ipython_input_max" \
    "$ipython_submit_avg" "$ipython_submit_min" "$ipython_submit_max" \
    "$ipython_result_avg" "$ipython_result_min" "$ipython_result_max" \
    "$ipython_submit_to_result_avg"
  printf '\nRelative slowdown\n'
  printf 'fpy input_avg vs ipython: %sx\n' "$input_ratio"
  printf 'fpy submit_avg vs ipython: %sx\n' "$submit_ratio"
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
    printf '  sample %s: input=%sms submit=%sms first_result=%sms submit_to_result=%sms\n' \
      "$1" "$2" "$3" "$4" "$5" >&2
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
