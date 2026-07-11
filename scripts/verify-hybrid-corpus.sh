#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPORT="${TCLD_CORPUS_REPORT:-${CARGO_TARGET_DIR:-$ROOT/target}/hybrid-corpus-report.json}"

cd "$ROOT"
mkdir -p "$(dirname "$REPORT")"
TCLD_CORPUS_REPORT="$REPORT" \
  cargo test -p tcld-vm --features host,debug \
    --test hybrid_corpus --test hybrid_fixtures -- --nocapture

test -s "$REPORT"
printf 'hybrid corpus report: %s\n' "$REPORT"
command cat "$REPORT"
printf '\n'
