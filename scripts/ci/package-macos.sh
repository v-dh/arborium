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

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
APP_NAME="Arbor"
APP_DIR="${OUTPUT_DIR}/${APP_NAME}.app"
CONTENTS_DIR="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"
ICONSET_DIR="${OUTPUT_DIR}/AppIcon.iconset"

is_lfs_pointer() {
  local path="$1"
  [[ -f "$path" ]] && head -n 1 "$path" | grep -Fqx 'version https://git-lfs.github.com/spec/v1'
}

mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}" "${ICONSET_DIR}"
install -m 0755 "${BINARY_PATH}" "${MACOS_DIR}/${APP_NAME}"

# Bundle arbor-httpd alongside the main binary so the GUI can auto-start it.
HTTPD_PATH="$(dirname "${BINARY_PATH}")/arbor-httpd"
if [[ -f "${HTTPD_PATH}" ]]; then
  install -m 0755 "${HTTPD_PATH}" "${MACOS_DIR}/arbor-httpd"
  echo "bundled arbor-httpd from ${HTTPD_PATH}"
else
  echo "note: arbor-httpd not found at ${HTTPD_PATH}, skipping bundle"
fi

# Bundle arbor-mcp alongside the main binary so packaged installs can expose
# Arbor as an MCP server without a separate build.
MCP_PATH="$(dirname "${BINARY_PATH}")/arbor-mcp"
if [[ -f "${MCP_PATH}" ]]; then
  install -m 0755 "${MCP_PATH}" "${MACOS_DIR}/arbor-mcp"
  echo "bundled arbor-mcp from ${MCP_PATH}"
else
  echo "note: arbor-mcp not found at ${MCP_PATH}, skipping bundle"
fi

# Bundle arbor CLI for scripting and automation.
CLI_PATH="$(dirname "${BINARY_PATH}")/arbor"
if [[ -f "${CLI_PATH}" ]]; then
  install -m 0755 "${CLI_PATH}" "${MACOS_DIR}/arbor"
  echo "bundled arbor CLI from ${CLI_PATH}"
else
  echo "note: arbor CLI not found at ${CLI_PATH}, skipping bundle"
fi

# Bundle web UI assets for arbor-httpd.
WEB_UI_DIST="${ROOT_DIR}/crates/arbor-web-ui/app/dist"
if [[ -d "${WEB_UI_DIST}" ]]; then
  cp -R "${WEB_UI_DIST}" "${RESOURCES_DIR}/web-ui"
  echo "bundled web-ui assets from ${WEB_UI_DIST}"
else
  echo "warning: web-ui dist not found at ${WEB_UI_DIST}, skipping bundle"
fi

# Bundle fonts for the GUI.
FONTS_DIR="${ROOT_DIR}/assets/fonts"
if [[ -d "${FONTS_DIR}" ]]; then
  mkdir -p "${RESOURCES_DIR}/fonts"
  for font_path in "${FONTS_DIR}"/*.ttf; do
    if is_lfs_pointer "${font_path}"; then
      echo "error: font asset is a Git LFS pointer, run 'git lfs pull': ${font_path}" >&2
      exit 1
    fi
  done
  cp "${FONTS_DIR}"/*.ttf "${RESOURCES_DIR}/fonts/"
  echo "bundled fonts from ${FONTS_DIR}"
else
  echo "warning: fonts not found at ${FONTS_DIR}, skipping bundle"
fi

# Bundle icon assets used by the GUI.
ICONS_DIR="${ROOT_DIR}/assets/icons"
if [[ -d "${ICONS_DIR}" ]]; then
  cp -R "${ICONS_DIR}" "${RESOURCES_DIR}/icons"
  echo "bundled icon assets from ${ICONS_DIR}"
else
  echo "warning: icons not found at ${ICONS_DIR}, skipping bundle"
fi

cp "${ROOT_DIR}/packaging/macos/Info.plist" "${CONTENTS_DIR}/Info.plist"
printf 'APPL????' > "${CONTENTS_DIR}/PkgInfo"

/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString ${TAG}" "${CONTENTS_DIR}/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion ${TAG}" "${CONTENTS_DIR}/Info.plist"

ICON_SOURCE="${ROOT_DIR}/assets/icons/arbor-icon-1024.png"
if is_lfs_pointer "${ICON_SOURCE}"; then
  echo "error: icon asset is a Git LFS pointer, run 'git lfs pull': ${ICON_SOURCE}" >&2
  exit 1
fi
for size in 16 32 128 256 512; do
  sips -z "${size}" "${size}" "${ICON_SOURCE}" --out "${ICONSET_DIR}/icon_${size}x${size}.png" >/dev/null
  double_size=$((size * 2))
  sips -z "${double_size}" "${double_size}" "${ICON_SOURCE}" --out "${ICONSET_DIR}/icon_${size}x${size}@2x.png" >/dev/null
done

iconutil -c icns "${ICONSET_DIR}" -o "${RESOURCES_DIR}/AppIcon.icns"
rm -rf "${ICONSET_DIR}"

ARCHIVE_NAME="${APP_NAME}-${TAG}-${TARGET_TRIPLE}.app.zip"
/usr/bin/ditto -c -k --keepParent --norsrc "${APP_DIR}" "${OUTPUT_DIR}/${ARCHIVE_NAME}"

echo "${OUTPUT_DIR}/${ARCHIVE_NAME}"
