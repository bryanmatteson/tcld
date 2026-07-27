#!/usr/bin/env bash
# Verify that private fixtures and missing third-party notices cannot slip into
# a public source distribution.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

forbidden_names=(
  soc.tbc
  soc.tcl
  soc.tbc.disassembly.txt
  n.tbc
)

for name in "${forbidden_names[@]}"; do
  if git ls-files | awk -F/ -v name="$name" '$NF == name { found=1 } END { exit !found }'; then
    echo "public-repository check failed: forbidden tracked file $name" >&2
    exit 1
  fi
  if git rev-list --objects HEAD |
    awk -v name="$name" 'NF > 1 { n=$0; sub(/^.* /, "", n); count=split(n, p, "/"); if (p[count] == name) found=1 } END { exit !found }'; then
    echo "public-repository check failed: forbidden historical file $name" >&2
    exit 1
  fi
done

required_paths=(
  LICENSE
  THIRD_PARTY_NOTICES.md
  testing/fixtures/public/public_complex_fixture.tcl
  testing/fixtures/public/public_complex_fixture.tbc
  apps/tcld/tests/fixtures/license.terms
  testing/fixtures/licenses/tclpro-corpus.terms
  testing/fuzz/corpus/license.terms
)

for path in "${required_paths[@]}"; do
  if [[ ! -f "$path" ]]; then
    echo "public-repository check failed: missing $path" >&2
    exit 1
  fi
done

echo "public-repository check passed"
