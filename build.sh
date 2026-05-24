#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

APP_BUNDLE="$ROOT_DIR/src-tauri/target/release/bundle/macos/seg.app"

if ! command -v npm >/dev/null 2>&1; then
  echo "error: npm is required to build seg" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required to build seg" >&2
  exit 1
fi

if [ ! -d node_modules ]; then
  echo "Installing npm dependencies..."
  npm ci
fi

echo "Checking Svelte/TypeScript..."
npm run check

echo "Building seg.app..."
npm run tauri -- build --bundles app

if [ ! -d "$APP_BUNDLE" ]; then
  echo "error: expected app bundle was not created at $APP_BUNDLE" >&2
  exit 1
fi

echo "Built: $APP_BUNDLE"
