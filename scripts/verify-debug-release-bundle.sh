#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <unpacked-cli-bundle>" >&2
  exit 2
fi

bundle=$1
target_triple=${TCLD_BUNDLE_TARGET:-}

verify_one() {
  local abi=$1
  local shell=${2:-}
  local directory="$bundle/lib/tcld/nub/$abi"
  local manifest="$directory/nub-manifest.json"
  local library
  library=$(find "$directory" -maxdepth 1 -type f \( -name 'libtcld_dbg.dylib' -o -name 'libtcld_dbg.so' \) -print -quit)
  test -n "$library"
  test -f "$manifest"
  python3 - "$library" "$manifest" "$abi" "$target_triple" <<'PY'
import hashlib
import json
import pathlib
import sys

library = pathlib.Path(sys.argv[1])
manifest_path = pathlib.Path(sys.argv[2])
abi = sys.argv[3]
target = sys.argv[4]
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
expected_keys = {
    "schema_version", "tcl_abi", "wire_protocol_major", "target_triple",
    "library_filename", "sha256",
}
if set(manifest) != expected_keys:
    raise SystemExit(f"manifest keys mismatch: {set(manifest)!r}")
if manifest["schema_version"] != 1 or manifest["wire_protocol_major"] != 2:
    raise SystemExit("manifest schema/protocol mismatch")
if manifest["tcl_abi"] != abi:
    raise SystemExit("manifest ABI mismatch")
if target and manifest["target_triple"] != target:
    raise SystemExit("manifest target mismatch")
if manifest["library_filename"] != library.name:
    raise SystemExit("manifest filename mismatch")
actual = hashlib.sha256(library.read_bytes()).hexdigest()
if manifest["sha256"].lower() != actual:
    raise SystemExit("manifest checksum mismatch")
PY

  if [[ "$(uname -s)" == Darwin ]]; then
    if otool -L "$library" | grep -E 'libtcl(8|9|\.)|Cellar|/opt/homebrew|/usr/local'; then
      echo "forbidden Tcl/build-machine linkage in $library" >&2
      exit 1
    fi
  elif command -v readelf >/dev/null; then
    if readelf -d "$library" | grep -E 'NEEDED.*libtcl(8|9|\.)|RPATH.*(homebrew|/usr/local)'; then
      echo "forbidden Tcl/build-machine linkage in $library" >&2
      exit 1
    fi
  fi

  if [[ -n "$shell" ]]; then
    printf 'load {%s} Tcld_dbg\nputs [package require tcld_dbg]\n' "$library" | "$shell" | grep -E '^0\.'
  fi
}

verify_one tcl8.6 "${TCLD_TCLSH86:-}"
verify_one tcl9.0 "${TCLD_TCLSH9:-}"
# Tcl 8.5 nubs are optional in release archives (packaged when a 8.5
# loadable build is available for the target). Verify when present.
if [[ -d "$bundle/lib/tcld/nub/tcl8.5" ]]; then
  verify_one tcl8.5 "${TCLD_TCLSH85:-}"
fi

binary=""
for candidate in "$bundle/tcld" "$bundle/bin/tcld"; do
  if [[ -x "$candidate" ]]; then
    binary=$candidate
    break
  fi
done

verify_cli_debug() {
  local shell=$1
  local abi=$2
  local mode=${3:-explicit}
  local supplied_target=${4:-}
  python3 - "$binary" "$shell" "$abi" "$mode" "$supplied_target" <<'PY'
import fcntl
import os
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

binary = os.path.abspath(sys.argv[1])
shell = os.path.abspath(sys.argv[2])
abi = sys.argv[3]
mode = sys.argv[4]
supplied_target = sys.argv[5]
extension = "dylib" if sys.platform == "darwin" else "so"
expected_nub = os.path.realpath(
    os.path.join(os.path.dirname(binary), "lib", "tcld", "nub", abi, f"libtcld_dbg.{extension}")
)

def descendants(parent):
    listing = subprocess.run(
        ["ps", "-axo", "pid=,ppid=,command="],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    rows = []
    for line in listing.splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) >= 2:
            rows.append((int(fields[0]), int(fields[1]), fields[2] if len(fields) == 3 else ""))
    found = []
    frontier = [parent]
    while frontier:
        owner = frontier.pop()
        for pid, ppid, command in rows:
            if ppid == owner and pid not in found:
                found.append(pid)
                frontier.append(pid)
    return found

def maps_expected_nub(pid):
    if sys.platform == "darwin":
        result = subprocess.run(
            ["lsof", "-Fn", "-p", str(pid)],
            check=False,
            capture_output=True,
            text=True,
        )
        return expected_nub in result.stdout
    try:
        with open(f"/proc/{pid}/maps", encoding="utf-8") as handle:
            return expected_nub in handle.read()
    except (FileNotFoundError, PermissionError):
        return False

def drain(fd, output):
    while True:
        try:
            chunk = os.read(fd, 65536)
        except (BlockingIOError, OSError):
            return
        if not chunk:
            return
        output.extend(chunk)

with tempfile.TemporaryDirectory(prefix="tcld-release-debug-") as root:
    target = os.path.abspath(supplied_target) if supplied_target else os.path.join(root, "target.tcl")
    home = os.path.join(root, "home")
    os.mkdir(home)
    if not supplied_target:
        with open(target, "w", encoding="utf-8") as handle:
            handle.write("set release_bundle_probe 1\n")

    search_path = None
    if mode == "auto":
        shell_bin = os.path.join(root, "shells")
        os.mkdir(shell_bin)
        os.symlink(os.path.abspath(os.environ["TCLD_TCLSH86"]), os.path.join(shell_bin, "tclsh8.6"))
        os.symlink(os.path.abspath(os.environ["TCLD_TCLSH9"]), os.path.join(shell_bin, "tclsh9.0"))
        search_path = os.pathsep.join([shell_bin, "/usr/bin", "/bin"])

    pid, fd = pty.fork()
    if pid == 0:
        env = os.environ.copy()
        for name in ("TCLD_DBG_DYLIB", "TCLD_TCLSH", "TCL_PREFIX", "TCL8_6_PREFIX", "TCL9_PREFIX", "DYLD_LIBRARY_PATH", "LD_LIBRARY_PATH"):
            env.pop(name, None)
        env.update({"TERM": "xterm-256color", "HOME": home})
        if search_path is not None:
            env["PATH"] = search_path
        os.chdir(root)
        args = [binary, "debug", target, "--no-colors"]
        if mode == "explicit":
            args.extend(["--tclsh", shell])
        os.execve(binary, args, env)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
    fcntl.fcntl(fd, fcntl.F_SETFL, fcntl.fcntl(fd, fcntl.F_GETFL) | os.O_NONBLOCK)
    output = bytearray()
    deadline = time.monotonic() + 12
    passed = False
    while time.monotonic() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.2)
        if ready:
            drain(fd, output)
        if any(maps_expected_nub(child) for child in descendants(pid)):
            passed = True
            break

    try:
        os.write(fd, b"\x03")
    except OSError:
        pass
    stop_deadline = time.monotonic() + 5
    status = None
    while time.monotonic() < stop_deadline:
        drain(fd, output)
        done, status = os.waitpid(pid, os.WNOHANG)
        if done:
            break
        time.sleep(0.05)
    else:
        os.close(fd)
        try:
            os.killpg(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        kill_deadline = time.monotonic() + 3
        while time.monotonic() < kill_deadline:
            done, status = os.waitpid(pid, os.WNOHANG)
            if done:
                break
            time.sleep(0.05)

    if not passed:
        sys.stderr.write(output.decode("utf-8", "replace"))
        raise SystemExit(f"packaged tcld debug did not map {expected_nub} with {abi}")
    if status is None:
        raise SystemExit(f"packaged tcld debug did not exit after verification with {abi}")
    if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
        raise SystemExit(f"packaged tcld debug exited abnormally with {abi}: {status}")
    print(f"packaged tcld debug ({mode}) mapped {expected_nub} with {abi}")
PY
}

if [[ -z "$binary" ]]; then
  echo "packaged tcld executable not found under $bundle" >&2
  exit 1
fi
if [[ "$(uname -s)" == Darwin ]]; then
  if otool -L "$binary" | grep -E 'libtcl(8|9|\.)|/Cellar/tcl|/opt/homebrew/opt/tcl'; then
    echo "packaged tcld executable has a forbidden compile-time Tcl dependency" >&2
    exit 1
  fi
elif command -v readelf >/dev/null; then
  if readelf -d "$binary" | grep -E 'NEEDED.*libtcl(8|9|\.)|RPATH.*tcl'; then
    echo "packaged tcld executable has a forbidden compile-time Tcl dependency" >&2
    exit 1
  fi
fi
if [[ -n "${TCLD_TCLSH86:-}" ]]; then
  verify_cli_debug "$TCLD_TCLSH86" tcl8.6
fi
if [[ -n "${TCLD_TCLSH9:-}" ]]; then
  verify_cli_debug "$TCLD_TCLSH9" tcl9.0
fi
if [[ -n "${TCLD_TCLSH86:-}" && -n "${TCLD_TCLSH9:-}" && -n "${TCLD_TCL8_BYTECODE_FIXTURE:-}" ]]; then
  test -f "$TCLD_TCL8_BYTECODE_FIXTURE"
  verify_cli_debug "$TCLD_TCLSH86" tcl8.6 auto "$TCLD_TCL8_BYTECODE_FIXTURE"
  verify_cli_debug "$TCLD_TCLSH9" tcl9.0 auto
fi
