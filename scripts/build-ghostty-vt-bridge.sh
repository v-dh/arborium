#!/usr/bin/env bash
set -euo pipefail

if ! command -v zig >/dev/null 2>&1; then
  echo "error: zig is required on PATH" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DEFAULT_GHOSTTY_SRC="${REPO_ROOT}/vendor/ghostty"
GHOSTTY_SRC="${ARBOR_GHOSTTY_SRC:-${DEFAULT_GHOSTTY_SRC}}"
OUT_DIR="${ARBOR_GHOSTTY_BRIDGE_OUT_DIR:-${REPO_ROOT}/target/ghostty-vt-bridge}"
LIB_DIR="${OUT_DIR}/lib"
BUILD_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arbor-ghostty-vt-XXXXXX")"
STAGED_GHOSTTY_DIR="${BUILD_DIR}/ghostty"
trap 'rm -rf "${BUILD_DIR}"' EXIT

if [ ! -d "${GHOSTTY_SRC}" ]; then
  echo "error: Ghostty source does not exist: ${GHOSTTY_SRC}" >&2
  echo "hint: run 'git submodule update --init --recursive vendor/ghostty' or set ARBOR_GHOSTTY_SRC" >&2
  exit 1
fi

if [ ! -f "${GHOSTTY_SRC}/build.zig" ]; then
  echo "error: Ghostty source is missing build.zig: ${GHOSTTY_SRC}" >&2
  exit 1
fi

mkdir -p "${STAGED_GHOSTTY_DIR}" "${LIB_DIR}"
rm -f "${LIB_DIR}/libarbor_ghostty_vt_bridge".*

if command -v rsync >/dev/null 2>&1; then
  rsync -a --delete --exclude '.git' "${GHOSTTY_SRC}/" "${STAGED_GHOSTTY_DIR}/"
else
  cp -R "${GHOSTTY_SRC}/." "${STAGED_GHOSTTY_DIR}/"
  rm -rf "${STAGED_GHOSTTY_DIR}/.git"
fi

cp "${REPO_ROOT}/scripts/ghostty-vt/arbor_build.zig" "${STAGED_GHOSTTY_DIR}/arbor_build.zig"
cp "${REPO_ROOT}/scripts/ghostty-vt/arbor_bridge.zig" "${STAGED_GHOSTTY_DIR}/arbor_bridge.zig"

(
  cd "${STAGED_GHOSTTY_DIR}"
  zig build --build-file arbor_build.zig -Doptimize=ReleaseFast
)

cp "${STAGED_GHOSTTY_DIR}/zig-out/lib/"libarbor_ghostty_vt_bridge.* "${LIB_DIR}/"

echo "built Ghostty VT bridge in ${LIB_DIR}"
