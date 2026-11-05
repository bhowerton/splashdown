#!/usr/bin/env bash
# Builds a release DMG of Splashdown for macOS: a universal (Apple Silicon +
# Intel) app with the hardened runtime, signed, and notarized when Apple
# credentials are set.
#
#   scripts/package-mac.sh               # signed test build
#   NOTARIZE=1 scripts/package-mac.sh    # signed and notarized, for release
#
# Needs: Xcode 26+, `cargo install tauri-cli --locked`,
# `rustup target add aarch64-apple-darwin x86_64-apple-darwin`, and
# optionally `cargo install cargo-about` (refreshes third-party notices).
#
# Signing: set APPLE_SIGNING_IDENTITY, or the first "Developer ID
# Application" certificate in your keychain is used. Only Developer ID
# builds can be notarized and opened without warnings on other Macs; an
# "Apple Development" certificate works for your own testing.
#
# Notarization (NOTARIZE=1, Developer ID only; takes a few minutes, so test
# builds skip it): put APPLE_API_KEY, APPLE_API_ISSUER and APPLE_API_KEY_PATH
# in .notarize.env (see .notarize.env.example). Tauri notarizes the app;
# this script then notarizes the DMG, and both get their tickets stapled.
#
# Updates: with the updater signing key (UPDATER_KEY, default
# ~/.tauri/splashdown-updater.key; see CONTRIBUTING.md), Tauri also makes a
# signed Splashdown.app.tar.gz, and this script writes latest.json for the
# GitHub release. Everything to upload ends up in .../bundle/release/.
# NOTARIZE=1 builds require the key; test builds skip update files without it.
set -euo pipefail
cd "$(dirname "$0")/.."

NOTARIZE=${NOTARIZE:-0}
if [ "$NOTARIZE" = 1 ]; then
    # Notarization settings kept out of git (see .notarize.env.example).
    if [ -f .notarize.env ]; then
        set -a
        # shellcheck disable=SC1091
        . ./.notarize.env
        set +a
    fi
    if [ -z "${APPLE_API_KEY:-}" ] || [ -z "${APPLE_API_ISSUER:-}" ] || [ -z "${APPLE_API_KEY_PATH:-}" ]; then
        echo "error: NOTARIZE=1 needs APPLE_API_KEY, APPLE_API_ISSUER and APPLE_API_KEY_PATH (.notarize.env)" >&2
        exit 1
    fi
else
    # Tauri notarizes whenever these are set; keep test builds local.
    unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH
fi

command -v cargo >/dev/null || export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export PATH="$HOME/.cargo/bin:$PATH"

if command -v cargo-about >/dev/null; then
    echo "Refreshing third-party license notices…"
    cargo about generate about.hbs -o THIRD-PARTY-LICENSES.html
fi

if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
    # By SHA-1, not name: renewed certificates share a name, which codesign
    # rejects as ambiguous.
    APPLE_SIGNING_IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null |
        awk '/Developer ID Application/ {print $2; exit}')
fi
if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
    APPLE_SIGNING_IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null |
        awk '/Apple Development/ {print $2; exit}')
    [ -n "$APPLE_SIGNING_IDENTITY" ] &&
        echo "warning: no Developer ID certificate; signing with an Apple Development one (fine for testing, not for distribution)" >&2
fi
if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
    echo "warning: no signing certificate found; signing ad hoc" >&2
    APPLE_SIGNING_IDENTITY="-"
fi
export APPLE_SIGNING_IDENTITY

# Three programs ship next to the app (Tauri's "external binaries"): the
# helper that runs the built-in local text model; splashdown-mcp, the MCP
# server coding agents are set up with (Settings → Privacy → Coding
# agents); and splashdown-cli, the command-line remote control, built
# without the developer harness (--no-default-features: 3 MB, not 38) in a
# folder of its own so it doesn't trade features with the full build.
# They're added through a release-only config so plain `cargo build`
# doesn't need them.
mkdir -p app/src-tauri/binaries
for bin in splashdown-llm splashdown-mcp splashdown-cli; do
    args=(--release -p "$bin")
    dir=target
    if [ "$bin" = splashdown-cli ]; then
        args+=(--no-default-features --target-dir target/slim)
        dir=target/slim
    fi
    cargo build "${args[@]}" --target aarch64-apple-darwin
    cargo build "${args[@]}" --target x86_64-apple-darwin
    # Tauri builds each architecture separately (wanting that one's
    # program), then bundles the universal file.
    for arch in aarch64 x86_64; do
        cp "$dir/$arch-apple-darwin/release/$bin" \
            "app/src-tauri/binaries/$bin-$arch-apple-darwin"
    done
    lipo -create -output "app/src-tauri/binaries/$bin-universal-apple-darwin" \
        "$dir/aarch64-apple-darwin/release/$bin" \
        "$dir/x86_64-apple-darwin/release/$bin"
done

# The updater signing key. Its password is empty unless one was set when
# it was generated. The key itself never goes into the repository.
UPDATER_KEY=${UPDATER_KEY:-$HOME/.tauri/splashdown-updater.key}
UPDATER=1
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    if [ -f "$UPDATER_KEY" ]; then
        TAURI_SIGNING_PRIVATE_KEY=$(cat "$UPDATER_KEY")
        export TAURI_SIGNING_PRIVATE_KEY
        export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
    elif [ "$NOTARIZE" = 1 ]; then
        echo "error: release builds need the updater signing key at $UPDATER_KEY (or TAURI_SIGNING_PRIVATE_KEY)" >&2
        exit 1
    else
        echo "warning: no updater signing key at $UPDATER_KEY; building without update files" >&2
        UPDATER=0
    fi
fi
CONFIGS=(--config app/src-tauri/tauri.release.conf.json)
[ "$UPDATER" = 1 ] || CONFIGS+=(--config '{"bundle":{"createUpdaterArtifacts":false}}')

cargo tauri build --target universal-apple-darwin --bundles app,dmg "${CONFIGS[@]}"

BUNDLE=target/universal-apple-darwin/release/bundle
APP="$BUNDLE/macos/Splashdown.app"
DMG=$(ls -t "$BUNDLE"/dmg/*.dmg | head -1)
if [ "$NOTARIZE" = 1 ]; then
    echo "Notarizing the DMG…"
    xcrun notarytool submit "$DMG" --wait \
        --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER"
    xcrun stapler staple "$DMG"
fi

# Release files: the DMG for new installs, under a name without the version
# so the website's link (releases/latest/download/Splashdown-mac.dmg) never
# changes; and for the updater the signed
# app archive plus latest.json, which installed copies fetch from
# releases/latest/download/latest.json. The URL assumes the release is
# tagged v<version>.
VERSION=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$APP/Contents/Info.plist")
RELEASE="$BUNDLE/release"
rm -rf "$RELEASE"
mkdir -p "$RELEASE"
cp "$DMG" "$RELEASE/Splashdown-mac.dmg"
if [ "$UPDATER" = 1 ]; then
    ARCHIVE="Splashdown_${VERSION}_universal.app.tar.gz"
    cp "$BUNDLE/macos/Splashdown.app.tar.gz" "$RELEASE/$ARCHIVE"
    cp "$BUNDLE/macos/Splashdown.app.tar.gz.sig" "$RELEASE/$ARCHIVE.sig"
    URL="https://github.com/bhowerton/splashdown/releases/download/v$VERSION/$ARCHIVE"
    SIGNATURE=$(cat "$RELEASE/$ARCHIVE.sig")
    # One universal archive serves both architectures.
    cat >"$RELEASE/latest.json" <<EOF
{
  "version": "$VERSION",
  "notes": "See the release notes on GitHub.",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
    "darwin-aarch64": { "signature": "$SIGNATURE", "url": "$URL" },
    "darwin-x86_64": { "signature": "$SIGNATURE", "url": "$URL" }
  }
}
EOF
fi

EXE="$APP/Contents/MacOS/$(/usr/libexec/PlistBuddy -c "Print :CFBundleExecutable" "$APP/Contents/Info.plist")"

echo
echo "Checks:"
echo "  architectures: $(lipo -archs "$EXE")"
HELPER="$APP/Contents/MacOS/splashdown-llm"
[ -x "$HELPER" ] && codesign --verify --strict "$HELPER" &&
    echo "  local model helper: present, signed ($(lipo -archs "$HELPER"))"
# Captured first: `grep -q` stopping early would fail the pipe under pipefail.
links=$(otool -l "$EXE")
grep -A2 LC_LOAD_WEAK_DYLIB <<<"$links" | grep -q FoundationModels &&
    echo "  FoundationModels: weak-linked (runs on macOS 13–25)"
/usr/libexec/PlistBuddy -c "Print :LSUIElement" "$APP/Contents/Info.plist" >/dev/null &&
    echo "  Info.plist: menu bar only (LSUIElement)"
/usr/libexec/PlistBuddy -c "Print :NSMicrophoneUsageDescription" "$APP/Contents/Info.plist" >/dev/null &&
    echo "  Info.plist: microphone usage text present"
codesign --verify --strict "$APP" && echo "  signature: valid ($APPLE_SIGNING_IDENTITY)"
entitlements=$(codesign -d --entitlements - "$APP" 2>/dev/null)
grep -q audio-input <<<"$entitlements" &&
    echo "  entitlements: audio input"
if spctl --assess --type execute "$APP" 2>/dev/null; then
    echo "  Gatekeeper (app): accepted"
else
    echo "  Gatekeeper (app): not accepted (expected without NOTARIZE=1 and a Developer ID)"
fi
if spctl --assess --type open --context context:primary-signature "$DMG" 2>/dev/null; then
    echo "  Gatekeeper (DMG): accepted"
else
    echo "  Gatekeeper (DMG): not accepted (expected without NOTARIZE=1)"
fi
if [ "$UPDATER" = 1 ]; then
    echo "  update archive: signed ($ARCHIVE.sig), latest.json for v$VERSION"
else
    echo "  update archive: not made (no signing key)"
fi
echo
echo "DMG: $DMG"
echo "Release files: $RELEASE"
