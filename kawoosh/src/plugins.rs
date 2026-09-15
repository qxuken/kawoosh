//! The bundled Lua plugins — the extension API's acceptance test (mvp.md
//! Decision 8): if the API cannot express them, it is not done.

pub const BUNDLED: &[(&str, &str)] = &[("kawoosh:oil", include_str!("../lua/oil.lua"))];
