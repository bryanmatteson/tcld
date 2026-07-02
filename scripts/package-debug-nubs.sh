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

package_one() {
  local source=$1
  local abi=$2
  local destination="$bundle_root/lib/tcld/nub/$abi"
  local filename
  filename=$(basename "$source")
  mkdir -p "$destination"
  cp "$source" "$destination/$filename"
  chmod 755 "$destination/$filename"
  python3 - "$destination/$filename" "$destination/nub-manifest.json" "$abi" "$target_triple" <<'PY'
import hashlib
import json
import pathlib
import sys

library = pathlib.Path(sys.argv[1])
manifest = pathlib.Path(sys.argv[2])
abi = sys.argv[3]
target = sys.argv[4]
payload = {
    "schema_version": 1,
    "tcl_abi": abi,
    "wire_protocol_major": 2,
    "target_triple": target,
    "library_filename": library.name,
    "sha256": hashlib.sha256(library.read_bytes()).hexdigest(),
}
manifest.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY
}

package_one "$tcl86_library" tcl8.6
package_one "$tcl9_library" tcl9.0
if [[ -n "$tcl85_library" ]]; then
  package_one "$tcl85_library" tcl8.5
fi
