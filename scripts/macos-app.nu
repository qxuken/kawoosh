#!/usr/bin/env nu
# Kawoosh.app: the window's binary as a macOS app, so Finder, the Dock and
# Spotlight open it with no Terminal window — Finder opens a bare
# executable in Terminal.app, whatever the executable is. What goes in:
#
#   Contents/MacOS/kawoosh            the window (CFBundleExecutable)
#   Contents/MacOS/kawoosh-edit       a terminal's $EDITOR, found beside it
#   Contents/Resources/kawoosh.icns   the icon (CFBundleIconFile), from
#                                     assets/icons
#   Contents/Resources/fonts/         the bundled faces, found from the
#                                     binary (left out with --no-fonts)
#   Contents/Info.plist
#
# The binary is the same one `cargo run` builds; the CLI half works from
# inside the app too:
#   ln -s /Applications/Kawoosh.app/Contents/MacOS/kawoosh ~/.local/bin/
#
# The app is made whole in a folder beside and renamed into place, so a
# Kawoosh running from the one replaced — which macOS lets go on
# running — sees a whole new app where it was started from the moment
# there is one, and offers to relaunch into it (kawoosh/src/update.rs):
# a Kawoosh builds the Kawoosh it runs from.
#
# A command that fails stops the script: nushell makes an external's
# non-zero exit an error.

# Build Kawoosh.app, and print where it is.
#
# Without its fonts (--no-fonts) the app finds them in the source tree it
# was built from (`fonts_dir` in main.rs), else an Iosevka installed on
# the system, else it draws in the system's mono.
def main [
  out_dir?: path  # where Kawoosh.app goes (default: target/release)
  --no-fonts      # leave the 252 MB of faces out
] {
  let root = $env.FILE_PWD | path dirname
  let target = $env.CARGO_TARGET_DIR? | default ($root | path join target)
  let app = $out_dir | default ($target | path join release) | path join Kawoosh.app | path expand
  let manifest = $root | path join Cargo.toml
  # Cargo reads `.cargo/config.toml` — the registry kui's crates come
  # from — in the directory it runs in, not the manifest's.
  cd $root

  ^cargo build --release --manifest-path $manifest -p kawoosh --bin kawoosh --bin kawoosh-edit
  # `path+file:///…/kawoosh#0.0.1`, or `…#kawoosh@0.0.1`.
  let version = ^cargo pkgid --manifest-path $manifest -p kawoosh | str trim | str replace -r '.*[#@]' ''

  # Made in a folder beside — the same disk, so the renames at the end
  # are renames — and under the app's own name there: `codesign` and
  # Finder know a bundle by it. What an interrupted run left goes first.
  let parent = $app | path dirname
  mkdir $parent
  let beside = $parent | path join .Kawoosh.new
  rm -rf $beside
  let fresh = $beside | path join Kawoosh.app
  let contents = $fresh | path join Contents
  mkdir ($contents | path join MacOS) ($contents | path join Resources)
  for bin in [kawoosh kawoosh-edit] {
    cp ($target | path join release $bin) ($contents | path join MacOS)
  }
  cp ($root | path join assets icons kawoosh.icns) ($contents | path join Resources)
  if not $no_fonts {
    let fonts = $root | path join assets fonts
    let resources = $contents | path join Resources
    # Two hundred megabytes of faces: cloned where the disk is APFS,
    # which takes no space until one side changes; copied where it is
    # not. macOS's own `cp`, for `-c`.
    try { ^/bin/cp -Rc $fonts $resources e> /dev/null } catch { ^/bin/cp -R $fonts $resources }
  }

  let plist = $contents | path join Info.plist
  $'<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>                    <string>Kawoosh</string>
  <key>CFBundleDisplayName</key>             <string>Kawoosh</string>
  <key>CFBundleIdentifier</key>              <string>dev.qxuken.kawoosh</string>
  <key>CFBundleExecutable</key>              <string>kawoosh</string>
  <key>CFBundleIconFile</key>                <string>kawoosh</string>
  <key>CFBundlePackageType</key>             <string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key>   <string>6.0</string>
  <key>CFBundleShortVersionString</key>      <string>($version)</string>
  <key>CFBundleVersion</key>                 <string>($version)</string>
  <key>LSApplicationCategoryType</key>       <string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key>         <true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key> <true/>
</dict>
</plist>
' | save -f $plist
  ^plutil -lint -s $plist

  # Ad hoc: enough for this machine, which is where the app was built.
  # Handing it to another Mac takes a Developer ID and notarization.
  ^codesign --force --sign - ($contents | path join MacOS kawoosh-edit)
  ^codesign --force --sign - $fresh

  # The old app aside and the new one in: a Kawoosh running from the old
  # one keeps the files it has open, and finds the new ones by path.
  if ($app | path exists) {
    mv $app ($beside | path join old.app)
  }
  mv $fresh $app
  rm -rf $beside

  $app
}
