#!/usr/bin/env nu
# Kawoosh for Windows: the window's binary as a folder of its own, so
# Explorer and the Start menu open it with no console window — the
# binary is a GUI program (`windows_subsystem`). What goes in:
#
#   Kawoosh\kawoosh.exe        the window, its icon linked in (build.rs)
#   Kawoosh\kawoosh-edit.exe   a terminal's $EDITOR, found beside it
#   Kawoosh\kawoosh-update.exe what :relaunch puts the next one in with
#   Kawoosh\conpty.dll         the pseudo console, and the host a terminal's
#   Kawoosh\OpenConsole.exe    screen is rendered in (`console-host` below)
#   Kawoosh\fonts\             the bundled faces, found from the binary
#                              (left out with --no-fonts)
#
# The console host is Microsoft's own, newer than the one in Windows:
# `portable-pty` takes a `conpty.dll` beside the executable before the
# system's. Windows' renders a program's output in frames of its own, so
# a `cargo build`'s progress line came in two reads 10 to 20 ms apart,
# the cursor mid-line between them for a frame to show — jumping; this
# one passes the program's writes through, 0.05 to 0.2 ms apart
# (measured 2026-10-04, Windows 11 26300 against 1.24). They are taken
# from the NuGet package, one version, its hash checked, and kept in
# `target\conpty` so the next build asks for nothing.
#
# A Kawoosh running from the folder keeps it: Windows renames no folder
# with a file open in it, and a Kawoosh holds its fonts open. The new
# one is then left beside, whole, as Kawoosh.new with a `ready` file in
# it, and the running Kawoosh offers to relaunch into it
# (kawoosh/src/update.rs) — so a Kawoosh builds the Kawoosh it runs
# from. A folder built --no-fonts does move under its Kawoosh, whose
# executable alone Windows lets be renamed: the new one goes in place,
# and the running one offers the relaunch as for one written over.
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

  # Before the build: a package that cannot be had stops it early.
  let host = console-host $target

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
  for f in $host {
    cp $f $fresh
  }
  if not $no_fonts {
    cp -r $fonts $fresh
  }
  let old = $parent | path join $"($aside)(random chars --length 8)"
  let moved = if ($app | path exists) { try { mv $app $old; true } catch { false } } else { true }
  if $moved {
    mv $fresh $app
    # A Kawoosh running from a folder of no fonts lets it move: the
    # loader leaves a running executable free to rename, though not to
    # delete. That one stays aside until the Kawoosh quits — it offers
    # to relaunch into the one now in place — and goes at the next run.
    try { rm -rf $old } catch {
      print -e $"($old) stays until the Kawoosh running from it quits; the next run takes it out."
    }
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

# Microsoft.Windows.Console.ConPTY (MIT), the version shipped and its
# package's SHA-256. To move to another: both, and a build.
const CONPTY_VERSION = '1.24.261001001'
const CONPTY_SHA256 = '4d6aaddc1d2385c9f5897df28f33879f699f8f2783315d5204cf3d8c3616ac5f'

# `conpty.dll` and `OpenConsole.exe` for this machine's architecture, as
# paths under `target\conpty`: the package downloaded where it is not
# there already, checked against its hash either way, and unpacked.
def console-host [target: path]: nothing -> list<path> {
  let arch = match $nu.os-info.arch {
    'x86_64' => 'x64'
    'aarch64' => 'arm64'
    $a => { error make {msg: $"no console host is packaged for ($a)"} }
  }
  let dir = $target | path join conpty $CONPTY_VERSION
  # A `.zip` by name: `Expand-Archive` reads no other.
  let package = $dir | path join package.zip
  if not ($package | path exists) {
    mkdir $dir
    let id = 'microsoft.windows.console.conpty'
    http get --raw $"https://api.nuget.org/v3-flatcontainer/($id)/($CONPTY_VERSION)/($id).($CONPTY_VERSION).nupkg" | save --raw $package
  }
  let hash = open --raw $package | hash sha256
  if $hash != $CONPTY_SHA256 {
    rm $package
    error make {msg: $"the ConPTY package ($CONPTY_VERSION) is not the one named here: its SHA-256 is ($hash). Taken out; run this again, or see CONPTY_SHA256"}
  }
  let files = [
    ($dir | path join runtimes $"win-($arch)" native conpty.dll)
    ($dir | path join build native runtimes $arch OpenConsole.exe)
  ]
  if not ($files | all {|f| $f | path exists }) {
    with-env {KAWOOSH_ZIP: $package, KAWOOSH_DIR: $dir} {
      ^powershell -NoProfile -NonInteractive -Command 'Expand-Archive -Force -LiteralPath $env:KAWOOSH_ZIP -DestinationPath $env:KAWOOSH_DIR'
    }
  }
  $files
}
