#!/usr/bin/env bash
# Kawoosh.app: the window's binary as a macOS app, so Finder, the Dock and
# Spotlight open it with no Terminal window — Finder opens a bare
# executable in Terminal.app, whatever the executable is. What goes in:
#
#   Contents/MacOS/kawoosh        the window (CFBundleExecutable)
#   Contents/MacOS/kawoosh-edit   a terminal's $EDITOR, found beside it
#   Contents/Resources/fonts/     the bundled faces, found from the binary
#   Contents/Info.plist
#
# The binary is the same one `cargo run` builds; the CLI half works from
# inside the app too:
#   ln -s /Applications/Kawoosh.app/Contents/MacOS/kawoosh ~/.local/bin/
#
# Usage: scripts/macos-app.sh [OUT_DIR]      (default: target/release)
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
target=${CARGO_TARGET_DIR:-$root/target}
out=${1:-$target/release}
app=$out/Kawoosh.app

cargo build --release --manifest-path "$root/Cargo.toml" \
  -p kawoosh --bin kawoosh --bin kawoosh-edit
version=$(cargo pkgid --manifest-path "$root/Cargo.toml" -p kawoosh)
version=${version##*[#@]}

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$target/release/kawoosh" "$target/release/kawoosh-edit" "$app/Contents/MacOS/"
# Two hundred megabytes of faces: cloned where the disk is APFS, which
# takes no space until one side changes; copied where it is not.
cp -Rc "$root/assets/fonts" "$app/Contents/Resources/" 2>/dev/null ||
  cp -R "$root/assets/fonts" "$app/Contents/Resources/"

cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>                    <string>Kawoosh</string>
  <key>CFBundleDisplayName</key>             <string>Kawoosh</string>
  <key>CFBundleIdentifier</key>              <string>dev.qxuken.kawoosh</string>
  <key>CFBundleExecutable</key>              <string>kawoosh</string>
  <key>CFBundlePackageType</key>             <string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key>   <string>6.0</string>
  <key>CFBundleShortVersionString</key>      <string>$version</string>
  <key>CFBundleVersion</key>                 <string>$version</string>
  <key>LSApplicationCategoryType</key>       <string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key>         <true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key> <true/>
</dict>
</plist>
EOF
plutil -lint -s "$app/Contents/Info.plist"

# Ad hoc: enough for this machine, which is where the app was built.
# Handing it to another Mac takes a Developer ID and notarization.
codesign --force --sign - "$app/Contents/MacOS/kawoosh-edit"
codesign --force --sign - "$app"

echo "$app"
