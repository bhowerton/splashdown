#!/usr/bin/env bash
# Builds Splashdown as a macOS .app bundle and launches it.
#
# Running the bare binary from a terminal makes macOS attribute the
# Accessibility and Microphone permissions to the terminal app. As a bundle,
# they belong to Splashdown itself, the way real users will run it.
#
#   scripts/run-mac-app.sh               # release build
#   PROFILE=dev scripts/run-mac-app.sh   # faster build, slower app
#
# Signing: uses your first "Apple Development" identity if you have one, so
# macOS keeps permissions and Keychain access across rebuilds. Override with
# SPLASHDOWN_SIGN_IDENTITY=<SHA-1> (see `security find-identity -v -p codesigning`),
# or SPLASHDOWN_SIGN_IDENTITY=- for ad-hoc signing (permissions reset every build).
#
# Logs: target/macos/splashdown.log
set -euo pipefail
cd "$(dirname "$0")/.."

command -v cargo >/dev/null || export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

PROFILE=${PROFILE:-release}
TARGET_DIR=$([ "$PROFILE" = dev ] && echo debug || echo "$PROFILE")
BUNDLE_ID=app.splashdown
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT=target/macos
APP=$OUT/Splashdown.app
LOG=$OUT/splashdown.log

# The app, plus the helper that runs the built-in local text model.
cargo build -p splashdown -p splashdown-llm --profile "$PROFILE"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/$TARGET_DIR/splashdown" "$APP/Contents/MacOS/splashdown"
cp "target/$TARGET_DIR/splashdown-llm" "$APP/Contents/MacOS/splashdown-llm"
cp app/src-tauri/icons/icon.icns "$APP/Contents/Resources/icon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleName</key><string>Splashdown</string>
  <key>CFBundleDisplayName</key><string>Splashdown</string>
  <key>CFBundleExecutable</key><string>splashdown</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
</dict>
</plist>
PLIST
# Usage descriptions and LSUIElement come from the same file release builds
# use, so the two never drift apart.
/usr/libexec/PlistBuddy -c "Merge app/src-tauri/Info.plist" "$APP/Contents/Info.plist" >/dev/null

IDENTITY=${SPLASHDOWN_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | awk '/Apple Development/ {print $2; exit}')}
IDENTITY=${IDENTITY:--}
echo "Signing with: $([ "$IDENTITY" = - ] && echo ad-hoc || security find-identity -v -p codesigning | grep "$IDENTITY" | cut -d'"' -f2)"
# The helper first: the app's signature covers it as-is.
codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID.llm" "$APP/Contents/MacOS/splashdown-llm" 2>&1 | grep -v "replacing existing signature" || true
codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID" "$APP" 2>&1 | grep -v "replacing existing signature" || true

# A helper left from the previous build would keep the old model code.
pkill -x splashdown-llm 2>/dev/null || true

# Replace a running copy.
pkill -x splashdown 2>/dev/null && sleep 0.5 || true

: > "$LOG"
ENV_ARGS=()
[ -f .env.local ] && ENV_ARGS=(--env "SPLASHDOWN_ENV_FILE=$PWD/.env.local")
open "$APP" "${ENV_ARGS[@]}" --stdout "$LOG" --stderr "$LOG"

echo "Launched $APP"
echo "Logs: tail -f $LOG"
