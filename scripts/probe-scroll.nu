#!/usr/bin/env nu
# The editor's scrolling probe (`scroll_probe::ScrollProbe`): a release
# kawoosh opens FILE in a window, holds `j`, `<C-d>` and `<C-f>` from its
# top one press a frame, timing each frame, and closes — once per family
# in FAMILIES ("" is the bundled face). Run against settings with only
# the font in them and a state of its own, so nothing of yours is read or
# written; each family's frames go to OUT-DIR (a CSV and its `.txt`) and
# the sums here.

# Scroll a file in a window in each family and say what each frame cost.
def main [
  file?: path  # the file to scroll; the workspace's Rust sources joined, when none
  --families: list<string> = ["" "Victor Mono" "JetBrains Mono"]  # the faces to compare
  --size: int = 13  # font.size
  --out-dir: path = "scroll-probe"  # where the frames go
  --no-build  # run the binary already built
] {
  let out_dir = ($out_dir | path expand)
  cd ($env.FILE_PWD | path dirname)
  if not $no_build { cargo build --release -p kawoosh }
  mkdir $out_dir
  let tmp = (mktemp -d)
  let file = if $file == null {
    let big = $"($tmp)/big.rs"
    glob **/src/**/*.rs --exclude [target/**] | sort | each {|f| open --raw $f } | str join "\n" | save -f $big
    $big
  } else { $file | path expand }
  "" | save -f $"($tmp)/init.lua"
  for family in $families {
    let name = if $family == "" { "bundled" } else { $family | str lowercase | str replace -a " " "-" }
    $"return { font = { family = \"($family)\", size = ($size) } }" | save -f $"($tmp)/settings.lua"
    rm -f $"($tmp)/state.db"
    with-env {
      KAWOOSH_PROBE_SCROLL: $"($out_dir)/($name).csv"
      KAWOOSH_STATE: $"($tmp)/state.db"
      KAWOOSH_INIT: $"($tmp)/init.lua"
      KAWOOSH_SETTINGS: $"($tmp)/settings.lua"
    } {
      ^./target/release/kawoosh $file
    }
  }
  rm -rf $tmp
}
