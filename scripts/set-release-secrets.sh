#!/usr/bin/env bash
# Uploads the release signing secrets to the GitHub "release" environment,
# then turns on releases from tags (the CI_RELEASES variable). Values are
# read from your files and piped to `gh`; nothing is printed.
#
#   scripts/set-release-secrets.sh ~/Desktop/developer-id.p12
#
# The .p12 is your "Developer ID Application" certificate with its private
# key: Keychain Access → My Certificates → right-click it → Export, choose a
# password. Export only that one. Delete the file afterward.
#
# Also reads: .notarize.env (App Store Connect API key, as for
# NOTARIZE=1 scripts/package-mac.sh) and the updater key
# (UPDATER_KEY, default ~/.tauri/splashdown-updater.key).
# Needs `gh`, logged in as a maintainer of the repository.
set -euo pipefail
cd "$(dirname "$0")/.."

P12=${1:?usage: scripts/set-release-secrets.sh path/to/developer-id.p12}
[ -f "$P12" ] || { echo "error: no file at $P12" >&2; exit 1; }
UPDATER_KEY=${UPDATER_KEY:-$HOME/.tauri/splashdown-updater.key}
[ -f "$UPDATER_KEY" ] || { echo "error: no updater key at $UPDATER_KEY" >&2; exit 1; }
[ -f .notarize.env ] || { echo "error: no .notarize.env (see .notarize.env.example)" >&2; exit 1; }
set -a
# shellcheck disable=SC1091
. ./.notarize.env
set +a
for v in APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH; do
    [ -n "${!v:-}" ] || { echo "error: $v missing from .notarize.env" >&2; exit 1; }
done
APPLE_API_KEY_PATH=${APPLE_API_KEY_PATH/#\~/$HOME}
[ -f "$APPLE_API_KEY_PATH" ] || { echo "error: no App Store Connect key at APPLE_API_KEY_PATH" >&2; exit 1; }

REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
read -r -s -p "Password you chose when exporting the .p12: " P12_PASSWORD
echo

# Catch a wrong password or the wrong certificate before uploading.
subjects=$(openssl pkcs12 -in "$P12" -nokeys -passin fd:3 3<<<"$P12_PASSWORD" 2>/dev/null |
    grep -i '^subject' || true)
if [ -z "$subjects" ]; then
    echo "warning: couldn't read the .p12 here (wrong password, or a format this openssl can't open); uploading anyway" >&2
elif ! grep -q "Developer ID Application" <<<"$subjects"; then
    echo "error: the .p12 holds no \"Developer ID Application\" certificate" >&2
    exit 1
elif [ "$(grep -c "Developer ID Application" <<<"$subjects")" -gt 1 ] || grep -q "Apple Development" <<<"$subjects"; then
    echo "error: the .p12 holds other signing certificates too; export only the Developer ID one" >&2
    exit 1
fi

put() { gh secret set "$1" --env release -R "$REPO" >/dev/null; echo "  set $1"; }
echo "Uploading to $REPO, environment \"release\":"
base64 -i "$P12" | put APPLE_CERTIFICATE
printf '%s' "$P12_PASSWORD" | put APPLE_CERTIFICATE_PASSWORD
printf '%s' "$APPLE_API_KEY" | put APPLE_API_KEY
printf '%s' "$APPLE_API_ISSUER" | put APPLE_API_ISSUER
put APPLE_API_KEY_P8 <"$APPLE_API_KEY_PATH"
put TAURI_SIGNING_PRIVATE_KEY <"$UPDATER_KEY"
# Only when the key has a password; the workflow treats a missing secret as
# an empty one.
if [ -n "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]; then
    printf '%s' "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD" | put TAURI_SIGNING_PRIVATE_KEY_PASSWORD
fi

gh variable set CI_RELEASES --body true -R "$REPO" >/dev/null
echo "  set variable CI_RELEASES=true (tag pushes now build releases)"
echo
echo "Done. Delete $P12 now; GitHub keeps its own copy."
