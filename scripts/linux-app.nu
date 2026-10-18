#!/usr/bin/env nu
# Kawoosh for Linux: the window's binary laid out under a prefix as Linux
# keeps a program, so the desktop's launcher lists it and opens it with
# no terminal. What goes in:
#
#   bin/kawoosh                       the window, and the CLI half
#   bin/kawoosh-edit                  a terminal's $EDITOR, found beside it
#   share/kawoosh/fonts/              the bundled faces, found from the
#                                     binary as ../share/kawoosh/fonts
#                                     (left out with --no-fonts)
#   share/applications/kawoosh.desktop
#                                     the launcher's entry, written by the
#                                     binary itself (`kawoosh
#                                     --desktop-entry`): it runs that
#                                     binary, `Icon=kawoosh`, and declares
#                                     the MIME types the build's languages
#                                     are on this machine, for Open with
#   share/icons/hicolor/scalable/apps/kawoosh.svg
#   share/icons/hicolor/128x128/apps/kawoosh.png
#                                     the icon the entry names: the drawing
#                                     and its 128 px render (assets/icons)
#
# With --install the prefix is ~/.local: a user's own programs, no root.
# `~/.local/share` is where the desktop looks first for entries and icons
# (XDG_DATA_HOME); another prefix's `share` is looked in only when it is
# on XDG_DATA_DIRS — `/usr/local` is by default. For the CLI half on the
# PATH, `~/.local/bin` has to be on it; most distributions put it there
# when it exists.
#
# Each file goes in by a rename, never written over in place: Linux lets
# a running executable be renamed over but not written ("Text file
# busy"). A Kawoosh running from the prefix keeps the files it has open,
# sees its executable replaced and offers to relaunch into the new one
# (kawoosh/src/update.rs) — so a Kawoosh builds the Kawoosh it runs from.
#
# A command that fails stops the script: nushell makes an external's
# non-zero exit an error.

# Build Kawoosh into a prefix, and print the prefix.
#
# Without its fonts (--no-fonts) the app finds them in the source tree it
# was built from (`shipped_dir` in fonts.rs), else an Iosevka installed
# on the system, else it draws in the system's mono.
def main [
  prefix?: path  # where bin/ and share/ go (default: target/release/Kawoosh, or ~/.local with --install)
  --no-fonts     # leave the 252 MB of faces out
  --install      # into ~/.local, the desktop's caches of entries and icons refreshed
] {
  let root = $env.FILE_PWD | path dirname
  let target = $env.CARGO_TARGET_DIR? | default ($root | path join target)
  let default_prefix = if $install {
    $env.HOME | path join .local
  } else {
    $target | path join release Kawoosh
  }
  let prefix = $prefix | default $default_prefix | path expand
  let manifest = $root | path join Cargo.toml
  let fonts = $root | path join assets fonts
  let icons = $root | path join assets icons

  # A checkout without `git lfs pull` has the faces as pointers of a
  # hundred bytes: shipped, the app would draw in the system's mono.
  if not $no_fonts {
    let pointer = ls ($fonts | path join IosevkaNavcon) | where type == file | first | get name
    if (open --raw $pointer | into binary | bytes starts-with ('version https://git-lfs' | into binary)) {
      error make {msg: $"the fonts are Git LFS pointers: run `git lfs pull` in ($root), or pass --no-fonts"}
    }
  }

  ^cargo build --release --manifest-path $manifest -p kawoosh --bin kawoosh --bin kawoosh-edit

  let bin = $prefix | path join bin
  let share = $prefix | path join share
  let apps = $share | path join applications
  let hicolor = $share | path join icons hicolor
  mkdir $bin $apps ($hicolor | path join scalable apps) ($hicolor | path join 128x128 apps)

  for name in [kawoosh kawoosh-edit] {
    put ($target | path join release $name) ($bin | path join $name)
  }
  put ($icons | path join kawoosh-icon.svg) ($hicolor | path join scalable apps kawoosh.svg)
  put ($icons | path join kawoosh-128.png) ($hicolor | path join 128x128 apps kawoosh.png)

  # The faces as a folder, swapped in whole: the new one beside, the old
  # one aside, the new one into its place. A Kawoosh running keeps the
  # files it has open; what an interrupted run left goes first.
  let dir = $share | path join kawoosh
  let shipped = $dir | path join fonts
  let fresh = $dir | path join .fonts.new
  let old = $dir | path join .fonts.old
  mkdir $dir
  rm -rf $fresh $old
  if not $no_fonts {
    cp -r $fonts $fresh
  }
  if ($shipped | path exists) {
    mv $shipped $old
  }
  if not $no_fonts {
    mv $fresh $shipped
  }
  rm -rf $old

  # Written by the binary just put in place, so its Exec is that one.
  let entry = $apps | path join kawoosh.desktop
  let staged = $apps | path join .kawoosh.desktop.new
  ^($bin | path join kawoosh) --desktop-entry | save -f $staged
  mv -f $staged $entry
  if (which desktop-file-validate | is-not-empty) {
    ^desktop-file-validate $entry
  }

  if $install {
    # The launchers that keep caches read them again; the rest look.
    if (which update-desktop-database | is-not-empty) {
      ^update-desktop-database $apps
    }
    if (which gtk-update-icon-cache | is-not-empty) {
      # `-t`: ~/.local's hicolor has no index.theme of its own.
      ^gtk-update-icon-cache -f -t -q $hicolor
    }
    let on_path = $env.PATH | any {|p| ($p | path expand) == $bin }
    if not $on_path {
      print -e $"($bin) is not on the PATH: add it for `kawoosh` in a shell. The launcher has it already."
    }
  }

  $prefix
}

# `from` put at `to` by a rename: copied beside it, then moved over it.
def put [from: path, to: path] {
  let staged = $to | path dirname | path join $".($to | path basename).new"
  cp $from $staged
  mv -f $staged $to
}
