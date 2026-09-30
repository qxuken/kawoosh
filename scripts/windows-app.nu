#!/usr/bin/env nu
# Kawoosh for Windows: the window's binary as a folder of its own, so
# Explorer and the Start menu open it with no console window — the
# binary is a GUI program (`windows_subsystem`). What goes in:
#
#   Kawoosh\kawoosh.exe        the window, its icon linked in (build.rs)
#   Kawoosh\kawoosh-edit.exe   a terminal's $EDITOR, found beside it
#   Kawoosh\kawoosh-update.exe what :relaunch puts the next one in with
#   Kawoosh\fonts\             the bundled faces, found from the binary
#                              (left out with --no-fonts)
#
# A Kawoosh running from the folder keeps it: Windows renames no folder
# with a file open in it. The new one is then left beside, whole, as
# Kawoosh.new with a `ready` file in it, and the running Kawoosh offers
# to relaunch into it (kawoosh/src/update.rs) — so a Kawoosh builds the
# Kawoosh it runs from.
#
# With --install the folder goes to %LOCALAPPDATA%\Programs — a user's
# own programs, no administrator — and a Start menu shortcut opens it in
# the home directory. The CLI half works from the folder too; for it on
# the PATH, add the folder to the user's Path (Settings, "Edit
# environment variables for your account").
#
# A command that fails stops the script: nushell makes an external's
# non-zero exit an error.

# Build the Kawoosh folder, and print where it is.
#
# Without its fonts (--no-fonts) the app finds them in the source tree it
# was built from (`fonts_dir` in main.rs), else an Iosevka installed on
# the system, else it draws in the system's mono.
def main [
  out_dir?: path  # where the Kawoosh folder goes (default: target\release, or %LOCALAPPDATA%\Programs with --install)
  --no-fonts      # leave the 252 MB of faces out
  --install       # into %LOCALAPPDATA%\Programs, with a Start menu shortcut
] {
  let root = $env.FILE_PWD | path dirname
  let target = $env.CARGO_TARGET_DIR? | default ($root | path join target)
  let default_dir = if $install {
    $env.LOCALAPPDATA | path join Programs
  } else {
    $target | path join release
  }
  let app = $out_dir | default $default_dir | path join Kawoosh | path expand
  let manifest = $root | path join Cargo.toml
  # Cargo reads `.cargo/config.toml` — the registry kui's crates come
  # from — in the directory it runs in, not the manifest's.
  cd $root
  let fonts = $root | path join assets fonts

  # A checkout without `git lfs pull` has the faces as pointers of a
  # hundred bytes: shipped, the app would draw in the system's mono.
  if not $no_fonts {
    let pointer = ls ($fonts | path join IosevkaNavcon) | where type == file | first | get name
    if (open --raw $pointer | into binary | bytes starts-with ('version https://git-lfs' | into binary)) {
      error make {msg: $"the fonts are Git LFS pointers: run `git lfs pull` in ($root), or pass --no-fonts"}
    }
  }

  ^cargo build --release --manifest-path $manifest -p kawoosh --bin kawoosh --bin kawoosh-edit --bin kawoosh-update
  # `path+file:///…/kawoosh#0.0.1`, or `…#kawoosh@0.0.1`.
  let version = ^cargo pkgid --manifest-path $manifest -p kawoosh | str trim | str replace -r '.*[#@]' ''

  # Made beside the old folder, then swapped in by renames: the old one
  # aside, the new one into its place. A Kawoosh running from the folder
  # keeps the first rename from happening, whole, and the install stays
  # as it was; the new folder is then left beside with `ready` written
  # in it last — the version — for that Kawoosh to relaunch into. What a
  # swap left aside goes at the next run.
  let fresh = $"($app).new"
  let aside = $"($app | path basename).old-"
  let parent = $app | path dirname
  if ($parent | path exists) {
    let stale = ls -a $parent | where type == dir and ($it.name | path basename | str starts-with $aside) | get name
    for d in $stale {
      try { rm -rf $d }
    }
  }
  rm -rf $fresh
  mkdir $fresh
  for bin in [kawoosh kawoosh-edit kawoosh-update] {
    cp ($target | path join release $"($bin).exe") $fresh
  }
  if not $no_fonts {
    cp -r $fonts $fresh
  }
  let old = $parent | path join $"($aside)(random chars --length 8)"
  let moved = if ($app | path exists) { try { mv $app $old; true } catch { false } } else { true }
  if $moved {
    mv $fresh $app
    rm -rf $old
  } else {
    $version | save ($fresh | path join ready)
    print -e $"($app) is in use, so Kawoosh ($version) waits beside it in ($fresh).
A Kawoosh running from there offers to relaunch into it: :relaunch.
One built before it cannot: quit it and run this again."
  }

  if $install {
    # A shortcut is a COM object's to write; PowerShell has the COM.
    # What it needs goes in the environment, so no path is quoted into
    # the command.
    let shortcut = $env.APPDATA | path join Microsoft Windows 'Start Menu' Programs Kawoosh.lnk
    with-env {
      KAWOOSH_LNK: $shortcut
      KAWOOSH_EXE: ($app | path join kawoosh.exe)
      KAWOOSH_HOME: $env.USERPROFILE
      KAWOOSH_DESC: $"Kawoosh ($version)"
    } {
      ^powershell -NoProfile -NonInteractive -Command '
        $s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:KAWOOSH_LNK)
        $s.TargetPath = $env:KAWOOSH_EXE
        $s.IconLocation = "$env:KAWOOSH_EXE,0"
        $s.WorkingDirectory = $env:KAWOOSH_HOME
        $s.Description = $env:KAWOOSH_DESC
        $s.Save()
      '
    }
  }

  $app
}
