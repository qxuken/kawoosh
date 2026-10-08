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
#   Kawoosh\kawoosh.lib        what a native extension links against, and
#   Kawoosh\include\           the two headers it is built with (below)
#
# A native extension (docs/design/native.md) calls `kw_*` and `kui_*`
# from kawoosh.exe. Windows resolves a DLL's imports through an import
# library naming the module they come from, so kawoosh.exe exports both
# families (kawoosh/build.rs) and link.exe writes `kawoosh.lib` beside it;
# shipped with `kawoosh.h` and the `kui.h` of the kui it was built with,
# an extension builds from this folder alone:
#
#   clang -O2 -shared -I Kawoosh\include dupes.c Kawoosh\kawoosh.lib -o dupes.dll
#
# and loads into this kawoosh.exe — an import library names its module.
#
# With --install, Kawoosh is also registered as an editor, all of it
# under HKCU (`register-editor` below):
#
#   Classes\Applications\kawoosh.exe   Explorer's Open with, a type for each
#                                      extension `kawoosh --languages` lists
#   Classes\*\OpenWithList\kawoosh.exe then any file at all
#   Classes\Kawoosh.File, RegisteredApplications, Software\Kawoosh
#                                      Settings' Default apps, where a type
#                                      can be given to Kawoosh
#   Classes\Directory\shell\Kawoosh    "Open in Kawoosh" on a folder and in
#   Classes\Directory\Background\…     one, which lists it
#   …\CurrentVersion\App Paths\kawoosh.exe
#                                      `kawoosh` in Run and `start kawoosh`
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
  --install       # into %LOCALAPPDATA%\Programs, with a Start menu shortcut, registered as an editor
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
  # link.exe writes the import library beside the binary in `deps\`,
  # where cargo leaves it.
  let lib = $target | path join release deps kawoosh.lib
  if not ($lib | path exists) {
    error make {msg: $"no ($lib): kawoosh.exe exported nothing, so no native extension can link against it. See the build's warnings (kawoosh/build.rs)"}
  }
  cp $lib $fresh
  mkdir ($fresh | path join include)
  cp ($root | path join kawoosh include kawoosh.h) ($fresh | path join include)
  cp (kui-header $manifest) ($fresh | path join include)
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
    # The types from the binary just built: the one in place may be the
    # old one, waiting on its relaunch.
    register-editor ($app | path join kawoosh.exe) ($target | path join release kawoosh.exe)
  }

  $app
}

# Kawoosh as an editor for this user: in Open with for every type the
# build knows and then any file, in Default apps, on a folder's menu,
# and in Run; what Explorer opens goes to the window running already,
# when there is one. None takes a type from the app that is its default — as
# on macOS, `Alternate` — until one is given to Kawoosh in Settings.
# What an install before wrote goes first, so a type the build no
# longer knows is no longer offered.
def register-editor [exe: path, built: path] {
  # `d.ts` would be a key Windows never looks up: it matches by the
  # last extension only.
  let extensions = ^$built --languages | from json | get extensions | flatten
    | where {|e| $e =~ '^[A-Za-z0-9_+-]+$' } | uniq
  with-env {
    KAWOOSH_EXE: $exe
    KAWOOSH_EXTS: ($extensions | str join ' ')
  } {
    ^powershell -NoProfile -NonInteractive -Command r#'
      $hkcu = [Microsoft.Win32.Registry]::CurrentUser
      function Put($path, $name, $value) {
        $k = $hkcu.CreateSubKey($path); $k.SetValue($name, $value); $k.Close()
      }
      function Types($path, $value) {
        $k = $hkcu.CreateSubKey($path)
        foreach ($e in $env:KAWOOSH_EXTS -split ' ') { $k.SetValue(".$e", $value) }
        $k.Close()
      }
      $exe = $env:KAWOOSH_EXE
      $icon = "`"$exe`",0"
      # `--reuse`: into the Kawoosh running already, raised (main.rs).
      $open = "`"$exe`" --reuse `"%1`""
      $folder = "`"$exe`" --reuse `"%V`""
      $classes = 'Software\Classes'
      $app = "$classes\Applications\kawoosh.exe"
      $progid = "$classes\Kawoosh.File"
      $caps = 'Software\Kawoosh\Capabilities'
      $paths = 'Software\Microsoft\Windows\CurrentVersion\App Paths\kawoosh.exe'
      foreach ($k in $app, $progid, 'Software\Kawoosh', $paths,
          "$classes\*\OpenWithList\kawoosh.exe",
          "$classes\Directory\shell\Kawoosh",
          "$classes\Directory\Background\shell\Kawoosh") {
        $hkcu.DeleteSubKeyTree($k, $false)
      }

      Put $paths '' $exe
      Put $paths 'Path' (Split-Path $exe)

      Put $app 'FriendlyAppName' 'Kawoosh'
      Put "$app\DefaultIcon" '' $icon
      Put "$app\shell\open\command" '' $open
      Types "$app\SupportedTypes" ''
      $hkcu.CreateSubKey("$classes\*\OpenWithList\kawoosh.exe").Close()

      Put $progid '' 'Kawoosh document'
      Put "$progid\DefaultIcon" '' $icon
      Put "$progid\shell\open\command" '' $open
      Put $caps 'ApplicationName' 'Kawoosh'
      Put $caps 'ApplicationDescription' 'A modal editor with terminals'
      Put $caps 'ApplicationIcon' $icon
      Types "$caps\FileAssociations" 'Kawoosh.File'
      Put 'Software\RegisteredApplications' 'Kawoosh' $caps

      foreach ($k in "$classes\Directory\shell\Kawoosh", "$classes\Directory\Background\shell\Kawoosh") {
        Put $k '' 'Open in Kawoosh'
        Put $k 'Icon' $icon
        Put "$k\command" '' $folder
      }

      # SHCNE_ASSOCCHANGED: Explorer reads the associations again.
      Add-Type -Namespace Kawoosh -Name Shell -MemberDefinition '
        [DllImport("shell32.dll")]
        public static extern void SHChangeNotify(int e, uint f, IntPtr a, IntPtr b);
      '
      [Kawoosh.Shell]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
    '#
  }
}

# `kui.h` of the kui-ffi the build links: the header `kawoosh.h`
# includes, at the ABI kawoosh.exe was built with.
def kui-header [manifest: path]: nothing -> path {
  ^cargo metadata --format-version 1 --offline --manifest-path $manifest
    | from json | get packages | where name == 'kui-ffi' | first
    | get manifest_path | path dirname | path join include kui.h
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
