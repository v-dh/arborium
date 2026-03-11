#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 <tag> <target-triple> <binary-path> <output-dir>" >&2
  exit 1
fi

TAG="$1"
TARGET_TRIPLE="$2"
BINARY_PATH="$3"
OUTPUT_DIR="$4"

APP_NAME="Arbor"
STAGING_DIR="${OUTPUT_DIR}/${APP_NAME}-${TAG}-${TARGET_TRIPLE}"
ARCHIVE_PATH="${OUTPUT_DIR}/${APP_NAME}-${TAG}-${TARGET_TRIPLE}.tar.gz"

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

is_lfs_pointer() {
  local path="$1"
  [[ -f "$path" ]] && head -n 1 "$path" | grep -Fqx 'version https://git-lfs.github.com/spec/v1'
}

mkdir -p "${STAGING_DIR}/bin" "${STAGING_DIR}/share/arbor"
install -m 0755 "${BINARY_PATH}" "${STAGING_DIR}/bin/${APP_NAME}"
cp README.md "${STAGING_DIR}/README.md"

# Bundle arbor-httpd alongside the main binary.
HTTPD_PATH="$(dirname "${BINARY_PATH}")/arbor-httpd"
if [[ -f "${HTTPD_PATH}" ]]; then
  install -m 0755 "${HTTPD_PATH}" "${STAGING_DIR}/bin/arbor-httpd"
  echo "bundled arbor-httpd from ${HTTPD_PATH}"
else
  echo "note: arbor-httpd not found at ${HTTPD_PATH}, skipping bundle"
fi

# Bundle arbor-mcp alongside the main binary.
MCP_PATH="$(dirname "${BINARY_PATH}")/arbor-mcp"
if [[ -f "${MCP_PATH}" ]]; then
  install -m 0755 "${MCP_PATH}" "${STAGING_DIR}/bin/arbor-mcp"
  echo "bundled arbor-mcp from ${MCP_PATH}"
else
  echo "note: arbor-mcp not found at ${MCP_PATH}, skipping bundle"
fi

# Bundle arbor CLI for scripting and automation.
CLI_PATH="$(dirname "${BINARY_PATH}")/arbor"
if [[ -f "${CLI_PATH}" ]]; then
  install -m 0755 "${CLI_PATH}" "${STAGING_DIR}/bin/arbor"
  echo "bundled arbor CLI from ${CLI_PATH}"
else
  echo "note: arbor CLI not found at ${CLI_PATH}, skipping bundle"
fi

# Bundle web UI assets for arbor-httpd.
WEB_UI_DIST="${ROOT_DIR}/crates/arbor-web-ui/app/dist"
if [[ -d "${WEB_UI_DIST}" ]]; then
  cp -R "${WEB_UI_DIST}" "${STAGING_DIR}/share/arbor/web-ui"
  echo "bundled web-ui assets from ${WEB_UI_DIST}"
else
  echo "warning: web-ui dist not found at ${WEB_UI_DIST}, skipping bundle"
fi

# Bundle fonts for the GUI.
FONTS_DIR="${ROOT_DIR}/assets/fonts"
if [[ -d "${FONTS_DIR}" ]]; then
  mkdir -p "${STAGING_DIR}/share/arbor/fonts"
  for font_path in "${FONTS_DIR}"/*.ttf; do
    if is_lfs_pointer "${font_path}"; then
      echo "error: font asset is a Git LFS pointer, run 'git lfs pull': ${font_path}" >&2
      exit 1
    fi
  done
  cp "${FONTS_DIR}"/*.ttf "${STAGING_DIR}/share/arbor/fonts/"
  echo "bundled fonts from ${FONTS_DIR}"
else
  echo "warning: fonts not found at ${FONTS_DIR}, skipping bundle"
fi

# Bundle icon assets used by the GUI.
ICONS_DIR="${ROOT_DIR}/assets/icons"
if [[ -d "${ICONS_DIR}" ]]; then
  cp -R "${ICONS_DIR}" "${STAGING_DIR}/share/arbor/icons"
  echo "bundled icon assets from ${ICONS_DIR}"
else
  echo "warning: icons not found at ${ICONS_DIR}, skipping bundle"
fi

tar -C "${OUTPUT_DIR}" -czf "${ARCHIVE_PATH}" "$(basename "${STAGING_DIR}")"

echo "${ARCHIVE_PATH}"
