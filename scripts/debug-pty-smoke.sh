#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/debug-pty-smoke.sh

Builds an isolated loadable Tcl 8.5, 8.6, or 9 debugger nub, then drives the real
`tcld debug` Crossterm application under a scripted PTY via:
  cargo test -p tcld --features integration_tclsh \
    --test debug_pty_terminal_contract -- --nocapture --test-threads=1

Set TCLD_DEBUG_PTY_TCLSH to select a Tcl 8.5, 8.6, or 9 shell. The shell's
reported patchlevel chooses the nub ABI; filenames are not trusted. Normalized terminal
captures are written to TCLD_DEBUG_PTY_FRAME. If unset, the frame defaults to:
  ${TMPDIR:-/tmp}/tcld-debug-pty-frame.txt
EOF
}

case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
  "") ;;
  *)
    usage >&2
    exit 2
    ;;
esac

tclsh=${TCLD_DEBUG_PTY_TCLSH:-}
if [[ -z "$tclsh" ]]; then
  tclsh=$(command -v tclsh9.0 || true)
fi
if [[ -z "$tclsh" ]]; then
  tclsh=$(command -v tclsh8.6 || true)
fi
if [[ -z "$tclsh" ]]; then
  tclsh=$(command -v tclsh8.5 || true)
fi
if [[ -z "$tclsh" ]]; then
  tclsh=$(command -v tclsh || true)
fi
if [[ -z "$tclsh" ]]; then
  echo "Tcl 8.5, 8.6, or 9 shell not found; set TCLD_DEBUG_PTY_TCLSH" >&2
  exit 1
fi
if [[ "$tclsh" != */* ]]; then
  tclsh=$(command -v "$tclsh")
fi

patchlevel=$("$tclsh" <<'TCL'
puts -nonewline [info patchlevel]
TCL
)
case "$patchlevel" in
  8.5.*)
    abi=tcl85
    feature=tcl8_5
    if [[ -z "${TCL8_5_PREFIX:-}" ]]; then
      TCL8_5_PREFIX=$(cd "$(dirname "$tclsh")/.." && pwd)
      export TCL8_5_PREFIX
    fi
    ;;
  8.6.*)
    abi=tcl86
    feature=tcl8_6
    if [[ -z "${TCL8_6_PREFIX:-}" ]]; then
      TCL8_6_PREFIX=$(cd "$(dirname "$tclsh")/.." && pwd)
      export TCL8_6_PREFIX
    fi
    ;;
  9.*)
    abi=tcl9
    feature=tcl9
    if [[ -z "${TCL9_PREFIX:-}" ]]; then
      TCL9_PREFIX=$(cd "$(dirname "$tclsh")/.." && pwd)
      export TCL9_PREFIX
    fi
    ;;
  *)
    echo "unsupported Tcl patchlevel '$patchlevel'; expected Tcl 8.5, 8.6, or 9" >&2
    exit 1
    ;;
esac

target_dir=${TCLD_DEBUG_PTY_TARGET_DIR:-target/debug-pty-nub-$abi}
cargo build -p tcld-dbg-ext --no-default-features \
  --features "$feature,loadable-extension" --target-dir "$target_dir"

extension=so
if [[ "$(uname -s)" == Darwin ]]; then
  extension=dylib
fi
dylib="$target_dir/debug/libtcld_dbg.$extension"
test -f "$dylib"
dylib="$(cd "$(dirname "$dylib")" && pwd)/$(basename "$dylib")"
tclsh="$(cd "$(dirname "$tclsh")" && pwd)/$(basename "$tclsh")"

tmp_root=${TMPDIR:-/tmp}
tmp_root=${tmp_root%/}
frame=${TCLD_DEBUG_PTY_FRAME:-$tmp_root/tcld-debug-pty-frame.txt}
rm -f "$frame"

printf 'Using Tcl shell: %s\n' "$tclsh"
printf 'Using Tcl patchlevel: %s (%s)\n' "$patchlevel" "$feature"
printf 'Using debugger nub: %s\n' "$dylib"
printf 'Writing normalized frame captures to: %s\n' "$frame"

TCLD_DEBUG_PTY_TCLSH="$tclsh" \
TCLD_DEBUG_PTY_DYLIB="$dylib" \
TCLD_DEBUG_PTY_FRAME="$frame" \
  cargo test -p tcld --features integration_tclsh \
    --test debug_pty_terminal_contract -- --nocapture

printf 'Debugger PTY frame captures: %s\n' "$frame"
