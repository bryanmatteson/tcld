#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 <library> <tcl-abi> <bundle-root> <target-triple>" >&2
  exit 2
fi

library=$1
abi=$2
bundle_root=$3
target_triple=$4

if [[ ! -f "$library" ]]; then
  echo "nub library not found: $library" >&2
  exit 1
fi
case "$abi" in
  tcl8.5|tcl8.6|tcl9.0) ;;
  *)
    echo "unsupported nub Tcl ABI: $abi" >&2
    exit 2
    ;;
esac

destination="$bundle_root/lib/tcld/nub/$abi"
filename=$(basename "$library")
mkdir -p "$destination"
cp "$library" "$destination/$filename"
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
