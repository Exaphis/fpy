#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
RAW_LOG="${RAW_LOG:-$ROOT/target/fpy-harness.raw.log}"
CLEAN_LOG="${CLEAN_LOG:-$ROOT/target/fpy-harness.clean.log}"
CMD="${FPY_CMD:-cargo run -- run --python .venv/bin/python}"
STARTUP_WAIT_MS="${STARTUP_WAIT_MS:-1500}"
INPUT_TEXT="${INPUT_TEXT:-}"
POST_CHECK="${POST_CHECK:-1}"

mkdir -p "$ROOT/target"

/usr/bin/expect \
  "$ROOT/scripts/fpy-harness.exp" \
  "$ROOT" \
  "$CMD" \
  "$RAW_LOG" \
  "$STARTUP_WAIT_MS" \
  "$INPUT_TEXT" \
  "$POST_CHECK"

perl -pe 's/\r\n/\n/g; s/\r/\n/g; s/\e\[[0-9;?]*[ -\/]*[@-~]//g; s/\e[@-_]//g' \
  "$RAW_LOG" > "$CLEAN_LOG"

printf 'raw log: %s\n' "$RAW_LOG"
printf 'clean log: %s\n' "$CLEAN_LOG"
