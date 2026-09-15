use std::path::Path;

use kawoosh::Kawoosh;
use kui::Core;

/// The bundled face, loaded onto a core the launcher then opens the
/// window on (`Launcher::core`): every mono run names it by `FontId`, so a
/// machine with no Iosevka installed draws the same glyphs.
fn load_fonts(core: &mut Core) -> Option<kui::FontId> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/fonts/IosevkaNavcon");
    let n = core.load_fonts_dir(&dir);
    log::info!("loaded {n} font faces from {}", dir.display());
    let family = core
        .system_font_families()
        .into_iter()
        .find(|f| f.contains("Iosevka"))?;
    core.add_system_font(&family)
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let path = std::env::args().nth(1);
    let mut core = Core::new();
    let font = load_fonts(&mut core);
    let mut app = match &path {
        Some(p) => Kawoosh::from_file(Path::new(p)),
        None => Kawoosh::new("*scratch*", SCRATCH),
    };
    app.font = font;
    kui::app("kawoosh")
        .size(1100.0, 760.0)
        .min_size(480.0, 320.0)
        .core(core)
        .run(app)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

const SCRATCH: &str = "\
kawoosh on kui — milestone 2

A modal editor over a selection set: h j k l w b e 0 ^ $ gg G, i a o O,
d c y with motions and text objects (dw, ciw, di(), v / V to select,
u and ctrl-r, / to search, : for commands (:w path, :q, :e file).
alt-j / alt-k add cursors; , keeps the primary.

Every visible line is a row of text runs; the row is the layout.
";
