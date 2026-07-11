#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
THIRD_PARTY_DIR="${ROOT_DIR}/third_party/tcl"
PATCH_FILE="${THIRD_PARTY_DIR}/patches/0001-opcode-runtime-hooks.patch"

TCL_GIT_URL="${TCL_GIT_URL:-https://github.com/tcltk/tcl.git}"
TCL_GIT_REF="${TCL_GIT_REF:-core-9-0-2}"
SRC_DIR="${THIRD_PARTY_DIR}/src/tcl-${TCL_GIT_REF}"
BUILD_DIR="${THIRD_PARTY_DIR}/build/tcl-${TCL_GIT_REF}"
PREFIX="${TCL_PREFIX:-${THIRD_PARTY_DIR}/runtime/tcl-${TCL_GIT_REF}}"

if [[ ! -f "${PATCH_FILE}" ]]; then
  echo "missing patch file: ${PATCH_FILE}" >&2
  exit 1
fi

mkdir -p "${THIRD_PARTY_DIR}/src" "${THIRD_PARTY_DIR}/build" "${THIRD_PARTY_DIR}/runtime"

if [[ ! -d "${SRC_DIR}/.git" ]]; then
  git clone "${TCL_GIT_URL}" "${SRC_DIR}"
fi

pushd "${SRC_DIR}" >/dev/null
git fetch --all --tags --prune
git checkout "${TCL_GIT_REF}"
git reset --hard "${TCL_GIT_REF}"
git clean -fd
git apply --index --3way --ignore-space-change --ignore-whitespace "${PATCH_FILE}" || {
  echo "failed to apply patch ${PATCH_FILE}" >&2
  exit 1
}
popd >/dev/null

rm -rf "${BUILD_DIR}"
mkdir -p "${BUILD_DIR}"
pushd "${BUILD_DIR}" >/dev/null
"${SRC_DIR}/unix/configure" \
  --prefix="${PREFIX}" \
  --enable-threads \
  --enable-symbols

if command -v nproc >/dev/null 2>&1; then
  JOBS="$(nproc)"
elif command -v sysctl >/dev/null 2>&1; then
  JOBS="$(sysctl -n hw.ncpu)"
else
  JOBS="4"
fi

make -j"${JOBS}"
make install
popd >/dev/null

echo "Built patched Tcl runtime at: ${PREFIX}"
echo "Set TCL9_PREFIX=${PREFIX} before building Rust crates that link Tcl9."
