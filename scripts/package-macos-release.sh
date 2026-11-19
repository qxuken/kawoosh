#!/usr/bin/env bash
set -euo pipefail
set +x

app="${1:?usage: package-macos-release.sh path/to/Kawoosh.app output.dmg}"
output="${2:?supply the output DMG path}"
: "${RUNNER_TEMP:?RUNNER_TEMP must name the runner temporary directory}"
for name in MACOS_CERTIFICATE_P12 MACOS_CERTIFICATE_PASSWORD MACOS_SIGN_IDENTITY \
    APPLE_ID APPLE_TEAM_ID APPLE_APP_PASSWORD; do
    if [[ -z "${!name:-}" ]]; then
        echo "Missing signing secret: $name" >&2
        exit 1
    fi
done
if [[ "$MACOS_SIGN_IDENTITY" != 'Developer ID Application: '* ]]; then
    echo 'MACOS_SIGN_IDENTITY must name a Developer ID Application certificate.' >&2
    exit 1
fi

umask 077
keychain_list=$(security list-keychains -d user)
original_keychains=()
while IFS= read -r path; do
    [[ -z "$path" ]] || original_keychains+=("$path")
done < <(printf '%s\n' "$keychain_list" | sed -E 's/^[[:space:]]*"//; s/"[[:space:]]*$//')
work_dir=$(mktemp -d "$RUNNER_TEMP/kawoosh-signing.XXXXXX")
keychain="$work_dir/signing.keychain-db"
cleanup() {
    security list-keychains -d user -s ${original_keychains[@]+"${original_keychains[@]}"} >/dev/null 2>&1 || true
    security delete-keychain "$keychain" >/dev/null 2>&1 || true
    rm -rf "$work_dir"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

keychain_password=$(openssl rand -hex 32)
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 7200 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
printf '%s' "$MACOS_CERTIFICATE_P12" | base64 --decode > "$work_dir/certificate.p12"
security import "$work_dir/certificate.p12" -k "$keychain" \
    -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
    -k "$keychain_password" "$keychain" >/dev/null
rm "$work_dir/certificate.p12"
unset MACOS_CERTIFICATE_P12 MACOS_CERTIFICATE_PASSWORD
security list-keychains -d user -s "$keychain" ${original_keychains[@]+"${original_keychains[@]}"}

xcrun notarytool store-credentials kawoosh-release --keychain "$keychain" \
    --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD"
unset APPLE_ID APPLE_TEAM_ID APPLE_APP_PASSWORD

# Restore normal file permissions for the disk image and its contents.
umask 022
stage="$work_dir/dmg"
bundle="$stage/Kawoosh.app"
mkdir "$stage"
ditto "$app" "$bundle"
ln -s /Applications "$stage/Applications"

sign=(codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" --keychain "$keychain")
"${sign[@]}" --options runtime --identifier dev.qxuken.kawoosh.edit \
    "$bundle/Contents/MacOS/kawoosh-edit"
"${sign[@]}" --options runtime --entitlements "$(dirname "$0")/macos-release-entitlements.plist" "$bundle"
codesign --verify --deep --strict --verbose=2 "$bundle"
"$bundle/Contents/MacOS/kawoosh" --version
"$bundle/Contents/MacOS/kawoosh-edit" --help

dmg="$work_dir/Kawoosh.dmg"
hdiutil create -quiet -volname Kawoosh -srcfolder "$stage" -fs HFS+ -format UDZO "$dmg"
"${sign[@]}" --identifier dev.qxuken.kawoosh.dmg "$dmg"

diagnostics="$RUNNER_TEMP/kawoosh-notarization"
mkdir -p "$diagnostics"
notary=(xcrun notarytool)
credentials=(--keychain "$keychain" --keychain-profile kawoosh-release)
submit_exit=0
"${notary[@]}" submit "$dmg" "${credentials[@]}" --wait --timeout 60m --output-format json \
    > "$diagnostics/submission.json" || submit_exit=$?
cat "$diagnostics/submission.json"
if submission_id=$(python3 -c 'import json, sys; print(json.load(sys.stdin)["id"])' < "$diagnostics/submission.json"); then
    "${notary[@]}" log "$submission_id" "${credentials[@]}" "$diagnostics/log.json" || true
fi
if [[ "$submit_exit" != 0 ]]; then
    exit "$submit_exit"
fi
python3 - "$diagnostics/submission.json" <<'PY'
import json
import sys

with open(sys.argv[1]) as response:
    status = json.load(response).get("status")
if status != "Accepted":
    sys.exit(f"Notarization was not accepted: {status}")
PY

xcrun stapler staple "$dmg"
xcrun stapler validate "$dmg"
codesign --verify --strict --verbose=2 "$dmg"
spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
mv "$dmg" "$output"
