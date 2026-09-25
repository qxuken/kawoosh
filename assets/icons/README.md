# Kawoosh's icons

The drawing is the user's (roadmap step 35).

| File | What it is | Where it goes |
|---|---|---|
| `kawoosh-icon.svg` | the icon, full bleed on a 100×100 rounded square | the source of `kawoosh.ico` |
| `kawoosh-icon-macos.svg` | the same on macOS's grid: inset to 80.5%, with the drop shadow | the source of `kawoosh.icns` |
| `kawoosh.icns` | exported from `kawoosh-icon-macos.svg` | Kawoosh.app's `Contents/Resources` (`scripts/macos-app.nu`) |
| `kawoosh.ico` | 16–256 px | linked into `kawoosh.exe` (`kawoosh/build.rs`, `kawoosh/kawoosh.rc`) |
| `kawoosh-128.png` | 128 px, rendered from `kawoosh-icon.svg` (`resvg -w 128 -h 128`) | the window's icon on X11, decoded at startup (`kawoosh/src/main.rs`) |
| `favicon.ico` | 16–64 px, exported from `kawoosh-icon.svg` | a web page's, when there is one |
| `kawoosh-mono.svg` | one colour, a plate under it | where colour is not wanted |
| `kawoosh-glyph.svg` | the glyph alone in `currentColor`, cropped to it | a tray or menu-bar item — a daemon's, later |

`kawoosh.ico` is `favicon.ico`'s four frames as exported, with 128 and
256 px rendered from `kawoosh-icon.svg` (`resvg -w 256 -h 256`) for
Explorer's large views; each frame a PNG. The windows carry the icon
too, through kui: `kawoosh.exe`'s resource on Windows, `kawoosh-128.png`
on X11. After changing a drawing, export again and rebuild.
