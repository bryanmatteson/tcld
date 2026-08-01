#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd "$script_dir/.." && pwd)
cd "$repo_root"

install_root=${TCLD_INSTALL_ROOT:-${CARGO_INSTALL_ROOT:-${CARGO_HOME:-${HOME:?HOME is required}/.cargo}}}
target_triple=$(rustc -vV | sed -n 's/^host: //p')
if [[ -z "$target_triple" ]]; then
  echo "could not determine the Rust host target triple" >&2
  exit 1
fi

extension=so
if [[ "$(uname -s)" == Darwin ]]; then
  extension=dylib
fi
nub_target_dir="$repo_root/target/install-nub-tcl9"

# Cargo installs binaries but not cdylib artifacts. Build and package the
# default Tcl 9 loadable nub explicitly so `tcld debug app.tcl` cannot discover
# an unrelated or stale checkout artifact after the CLI has been upgraded.
cargo build -p tcld-dbg-ext --no-default-features \
  --features tcl9,loadable-extension --target-dir "$nub_target_dir"
cargo install --force --locked --path apps/tcld --root "$install_root"
"$script_dir/package-debug-nub.sh" \
  "$nub_target_dir/debug/libtcld_dbg.$extension" \
  tcl9.0 "$install_root" "$target_triple"

echo "installed tcld and the Tcl 9 debugger nub under $install_root"
