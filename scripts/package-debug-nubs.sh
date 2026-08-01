#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 4 || $# -gt 5 ]]; then
  echo "usage: $0 <tcl86-library> <tcl9-library> <bundle-root> <target-triple> [tcl85-library]" >&2
  exit 2
fi

tcl86_library=$1
tcl9_library=$2
bundle_root=$3
target_triple=$4
tcl85_library=${5:-}

for library in "$tcl86_library" "$tcl9_library"; do
  if [[ ! -f "$library" ]]; then
    echo "nub library not found: $library" >&2
    exit 1
  fi
done
if [[ -n "$tcl85_library" && ! -f "$tcl85_library" ]]; then
  echo "nub library not found: $tcl85_library" >&2
  exit 1
fi

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

"$script_dir/package-debug-nub.sh" "$tcl86_library" tcl8.6 "$bundle_root" "$target_triple"
"$script_dir/package-debug-nub.sh" "$tcl9_library" tcl9.0 "$bundle_root" "$target_triple"
if [[ -n "$tcl85_library" ]]; then
  "$script_dir/package-debug-nub.sh" "$tcl85_library" tcl8.5 "$bundle_root" "$target_triple"
fi
