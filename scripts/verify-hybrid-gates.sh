#!/usr/bin/env bash
# verify-hybrid-gates.sh — production Hybrid-max gates for the hybrid TEBC path.
#
# Hybrid feature verification:
#   cargo build -p tcld --features hybrid
# Hybrid is **opt-in** so default `tcld` stays free of an embedded Tcl link.
# This script always builds/tests with `--features hybrid` and fails closed
# if Tcl 9 headers/libs are not available (no silent skip of Pass gates).
#
# Usage:
#   scripts/verify-hybrid-gates.sh
#   TCL9_PREFIX=/path/to/tcl scripts/verify-hybrid-gates.sh
#
# Exit codes:
#   0  all gates passed
#   1  build/test/run failure
#   2  environment missing (no Tcl 9 for hybrid link)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

log() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
die_env() { printf 'error: %s\n' "$*" >&2; exit 2; }

# ── Environment: require a Tcl 9 discovery path ───────────────────────
if [[ -z "${TCL9_PREFIX:-}" && -z "${TCL_PREFIX:-}" ]]; then
  # Best-effort local discovery (matches tcl-sys build heuristics).
  for cand in \
    /opt/homebrew/opt/tcl-tk \
    /opt/homebrew/opt/tcl-tk@9 \
    /usr/local/opt/tcl-tk \
    /usr; do
    if [[ -d "$cand/include" ]] || [[ -f "$cand/include/tcl.h" ]] || [[ -d "$cand/include/tcl-tk" ]]; then
      export TCL9_PREFIX="${TCL9_PREFIX:-$cand}"
      break
    fi
  done
fi

if [[ -z "${TCL9_PREFIX:-}" && -z "${TCL_PREFIX:-}" ]]; then
  if command -v tclsh9.0 >/dev/null 2>&1 || command -v tclsh9 >/dev/null 2>&1; then
    log "note: TCL9_PREFIX unset; relying on tcl-sys tclsh discovery"
  else
    die_env "Tcl 9 required for hybrid gates. Set TCL9_PREFIX (or TCL_PREFIX) to the Tcl 9 install prefix, or install tclsh9.0 on PATH."
  fi
fi

log "==> Hybrid-max production gates (feature: hybrid)"
log "    ROOT=$ROOT"
log "    TCL9_PREFIX=${TCL9_PREFIX:-<unset>}"
log "    TCL_PREFIX=${TCL_PREFIX:-<unset>}"

# ── 1. Domain + app contracts ─────────────────────────────────────────
log "==> cargo fmt --all -- --check"
cargo fmt --all -- --check

log "==> cargo test -p tcld-core"
cargo test -p tcld-core

log "==> cargo test -p tcld-vm --features host,debug"
cargo test -p tcld-vm --features host,debug

log "==> cargo test -p tcld-core -- get_instructions_raw"
cargo test -p tcld-core -- get_instructions_raw

log "==> hybrid CLI/debug/PTY contracts"
cargo test -p tcld --features hybrid \
  --test debug_planning_contract \
  --test hybrid_run_contract \
  --test hybrid_debug_contract \
  --test debug_pty_terminal_contract

log "==> pinned Hybrid-max corpus"
scripts/verify-hybrid-corpus.sh

# ── 2. Production binary ──────────────────────────────────────────────
log "==> cargo build -p tcld --features hybrid"
cargo build -p tcld --features hybrid

TCLD="${CARGO_TARGET_DIR:-${ROOT}/target}/debug/tcld"
[[ -x "$TCLD" ]] || die "hybrid binary missing at $TCLD"

# Structural I1: hybrid modules must not name decompile_to_temp
log "==> I1 structural (hybrid modules)"
if rg -n 'decompile_to_temp' \
  apps/tcld/src/run.rs \
  apps/tcld/src/debug/backend.rs \
  apps/tcld/src/debug/app.rs; then
  die "I1 violation: decompile_to_temp appears in hybrid modules"
fi
log "    OK (no decompile_to_temp in hybrid modules)"

# ── 3. Real entry points ──────────────────────────────────────────────
PROC="$ROOT/apps/tcld/tests/fixtures/proc.tbc"
BREAK="$ROOT/apps/tcld/tests/fixtures/break.tbc"
[[ -f "$PROC" ]] || die "missing fixture $PROC"
[[ -f "$BREAK" ]] || die "missing fixture $BREAK"

run_one() {
  local label="$1"
  shift
  log "==> $label"
  set +e
  out="$("$TCLD" "$@" 2>&1)"
  code=$?
  set -e
  printf '%s\n' "$out"
  [[ "$code" -eq 0 ]] || die "$label failed with exit $code"
}

run_one "tcld run proc.tbc" run "$PROC"
run_one "tcld run break.tbc" run "$BREAK"
run_one "tcld run --sandbox break.tbc" run --sandbox "$BREAK"

# ── 4. Feature isolation regressions ──────────────────────────────────
log "==> cargo build -p tcld (default, no embedded Tcl)"
cargo build -p tcld

log "==> cargo test -p tcld --no-default-features"
cargo test -p tcld --no-default-features

log "==> Hybrid-max gates OK"
exit 0
