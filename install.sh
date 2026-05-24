#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_BUNDLE="$ROOT_DIR/src-tauri/target/release/bundle/macos/seg.app"
INSTALL_DIR="${1:-/Applications}"
DEST="$INSTALL_DIR/seg.app"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "error: install.sh currently installs the macOS .app bundle and must be run on macOS" >&2
  exit 1
fi

if [ ! -d "$APP_BUNDLE" ]; then
  "$ROOT_DIR/build.sh"
fi

if [ ! -d "$APP_BUNDLE" ]; then
  echo "error: app bundle not found at $APP_BUNDLE" >&2
  exit 1
fi

if [ ! -d "$INSTALL_DIR" ]; then
  echo "Creating $INSTALL_DIR..."
  mkdir -p "$INSTALL_DIR"
fi

SUDO=""
if [ ! -w "$INSTALL_DIR" ]; then
  if ! command -v sudo >/dev/null 2>&1; then
    echo "error: $INSTALL_DIR is not writable and sudo is unavailable" >&2
    exit 1
  fi
  SUDO="sudo"
fi

echo "Installing $DEST..."
$SUDO rm -rf "$DEST"
$SUDO ditto "$APP_BUNDLE" "$DEST"

echo "Installed: $DEST"
