//! kawoosh in a browser (web/README.md): the window is a `<canvas>` kui
//! opens on the page, the disk is one held in memory and seeded with a
//! few of kawoosh's own files, and the rest is the editor as it is — its
//! Lua, its grammars, its keys. What a page cannot have is left out where
//! it would start: terminals (no processes), language servers, the
//! command socket, the state database.
//!
//! The page calls [`start`] once the module is instantiated and its WASI
//! imports are bound to its memory (web/www/index.html): the C library
//! Lua is built against writes through them.

#![cfg(target_arch = "wasm32")]

mod libc;
mod memfs;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kawoosh::Kawoosh;
use kawoosh::logger::Logger;
use kawoosh::notify::Level;
use kawoosh_systems::WakeHandle;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Where the demo's tree lives, and where the editor starts.
const ROOT: &str = "/kawoosh";

/// The demo's files: a few of kawoosh's own, at their places in the repo.
const FILES: &[(&str, &str)] = &[
    ("web/README.md", include_str!("../README.md")),
    ("web/src/lib.rs", include_str!("lib.rs")),
    ("web/src/memfs.rs", include_str!("memfs.rs")),
    ("Cargo.toml", include_str!("../../Cargo.toml")),
    (
        "editor/src/motions.rs",
        include_str!("../../editor/src/motions.rs"),
    ),
    (
        "editor/src/selection.rs",
        include_str!("../../editor/src/selection.rs"),
    ),
    ("doc/src/paths.rs", include_str!("../../doc/src/paths.rs")),
    (
        "kawoosh/lua/pairs.lua",
        include_str!("../../kawoosh/lua/pairs.lua"),
    ),
    (
        "kawoosh/lua/picker.lua",
        include_str!("../../kawoosh/lua/picker.lua"),
    ),
    (
        "kawoosh/lua/dirs.lua",
        include_str!("../../kawoosh/lua/dirs.lua"),
    ),
    (
        "docs/design/roadmap.md",
        include_str!("../../docs/design/roadmap.md"),
    ),
    (
        "docs/design/kui.md",
        include_str!("../../docs/design/kui.md"),
    ),
];

/// The faces fetched from the page (`fonts/` beside it), regular first:
/// the family kawoosh draws its mono runs in.
const FACES: &[&str] = &[
    "IosevkaNavcon-Regular.ttf",
    "IosevkaNavcon-Bold.ttf",
    "IosevkaNavcon-Italic.ttf",
    "IosevkaNavcon-BoldItalic.ttf",
];

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let resp: web_sys::Response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
        .await?
        .dyn_into()?;
    if !resp.ok() {
        return Err(format!("{url}: {}", resp.status()).into());
    }
    let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer()?).await?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

/// The page's disk: the demo's tree under [`ROOT`], a home beside it.
fn local_disk() {
    let home = PathBuf::from("/home/kawoosh");
    let fs = memfs::MemFs::new(home.clone());
    for (path, text) in FILES {
        fs.seed(&format!("{ROOT}/{path}"), text.as_bytes());
    }
    let _ = kawoosh_doc::fs::set_local_disk(kawoosh_doc::fs::LocalDisk {
        fs: Arc::new(fs),
        cwd: PathBuf::from(ROOT),
        home,
    });
}

/// Opens kawoosh on the page: its fonts fetched, its disk made, the
/// editor set up as `main` sets it up, and the window handed to kui.
#[wasm_bindgen]
pub async fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    local_disk();
    let mut faces = Vec::new();
    for face in FACES {
        faces.push(fetch_bytes(&format!("fonts/{face}")).await?);
    }

    let wake = WakeHandle::new();
    let log_sink = Logger::install(wake.clone(), Level::Debug);
    let mut core = kui::Core::new();
    for face in faces {
        core.add_font_data(face);
    }
    let font = core
        .system_font_families()
        .into_iter()
        .find(|f| f.contains("Iosevka"))
        .and_then(|family| core.add_system_font(&family));
    let mut app = Kawoosh::new("*scratch*", "");
    app.bundled_font = font;
    app.face.id = font;
    app.log_sink = log_sink;
    app.notes.stderr = None;
    app.notes.keep = Level::Debug;
    app.share_wake(wake);
    let ext = app
        .attach_lua()
        .map_err(|e| JsValue::from_str(&format!("lua: {e}")))?;
    app.load_config();
    app.open_first(Path::new(&format!("{ROOT}/web/README.md")));
    kui::app("kawoosh")
        .core(core)
        .extension_as("lua", ext)
        .run(app)
        .map_err(|e| JsValue::from_str(&e.to_string()))
}
