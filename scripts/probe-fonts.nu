#!/usr/bin/env nu
# The fonts pane's windowed probe (`fonts::Probe`, docs/design/fonts.md):
# a release kawoosh opens the pane on every family and walks it a card a
# frame to the last, timing each frame and noting the families it shaped
# first, then closes — a window on screen for the length of the walk.
# Run against empty settings and a state of its own, so nothing of yours
# is read or written; the table goes to OUT (a CSV) and the sums to
# OUT's `.txt` and here.

# Walk the fonts pane in a window and say what each frame cost.
def main [
  out: path = "fonts-probe.csv"  # where the frames go
  --no-build  # run the binary already built
] {
  let out = ($out | path expand)
  cd ($env.FILE_PWD | path dirname)
  if not $no_build { cargo build --release -p kawoosh }
  let tmp = (mktemp -d)
  "" | save -f $"($tmp)/init.lua"
  "return {}" | save -f $"($tmp)/settings.lua"
  with-env {
    KAWOOSH_PROBE_FONTS: $out
    KAWOOSH_STATE: $"($tmp)/state.db"
    KAWOOSH_INIT: $"($tmp)/init.lua"
    KAWOOSH_SETTINGS: $"($tmp)/settings.lua"
  } {
    ^./target/release/kawoosh $tmp
  }
  rm -rf $tmp
}
