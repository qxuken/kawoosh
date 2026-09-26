#!/usr/bin/env nu
# Kawoosh for Windows: the window's binary as a folder of its own, so
# Explorer and the Start menu open it with no console window — the
# binary is a GUI program (`windows_subsystem`). What goes in:
#
#   Kawoosh\kawoosh.exe        the window, its icon linked in (build.rs)
#   Kawoosh\kawoosh-edit.exe   a terminal's $EDITOR, found beside it
#   Kawoosh\fonts\             the bundled faces, found from the binary
#                              (left out with --no-fonts)
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

  ^cargo build --release --manifest-path $manifest -p kawoosh --bin kawoosh --bin kawoosh-edit
  # `path+file:///…/kawoosh#0.0.1`, or `…#kawoosh@0.0.1`.
  let version = ^cargo pkgid --manifest-path $manifest -p kawoosh | str trim | str replace -r '.*[#@]' ''

  # Made beside the old folder, then swapped in by renames: a running
  # kawoosh.exe holds its folder, so the old one's rename fails whole
  # and the install stays as it was — where a removal would have taken
  # every file but the running one.
  let fresh = $"($app).new"
  let old = $"($app).old"
  rm -rf $fresh $old
  mkdir $fresh
  for bin in [kawoosh kawoosh-edit] {
    cp ($target | path join release $"($bin).exe") $fresh
  }
  if not $no_fonts {
    cp -r $fonts $fresh
  }
  if ($app | path exists) {
    try { mv $app $old } catch {
      rm -rf $fresh
      error make {msg: $"cannot replace ($app): close Kawoosh if it is running from there"}
    }
  }
  mv $fresh $app
  rm -rf $old

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
