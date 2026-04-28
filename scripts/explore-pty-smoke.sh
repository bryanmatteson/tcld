#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/explore-pty-smoke.sh [INPUT]

Runs the real Crossterm explorer under a PTY via:
  cargo test -p tcld --test explore_pty_terminal_contract -- --nocapture

INPUT defaults to testing/fixtures/public/public_complex_fixture.tbc so the smoke crosses a real viewport boundary.

The normalized terminal captures are written to TCLD_EXPLORE_PTY_FRAME.
If unset, TCLD_EXPLORE_PTY_FRAME defaults to:
  ${TMPDIR:-/tmp}/tcld-explore-pty-frame.txt
EOF
}

case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
esac

input="${1:-testing/fixtures/public/public_complex_fixture.tbc}"
tmp_root="${TMPDIR:-/tmp}"
tmp_root="${tmp_root%/}"
frame="${TCLD_EXPLORE_PTY_FRAME:-$tmp_root/tcld-explore-pty-frame.txt}"
rm -f "$frame"

printf 'Running TUI PTY smoke against: %s\n' "$input"
printf 'Writing normalized frame captures to: %s\n' "$frame"

TCLD_EXPLORE_PTY_INPUT="$input" \
TCLD_EXPLORE_PTY_FRAME="$frame" \
  cargo test -p tcld --test explore_pty_terminal_contract -- --nocapture

printf 'TUI PTY frame captures: %s\n' "$frame"
