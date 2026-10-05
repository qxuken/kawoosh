//! The bundled Lua plugins — the extension API's acceptance test (mvp.md
//! Decision 8): if the API cannot express them, it is not done.

pub const BUNDLED: &[(&str, &str)] = &[
    ("kawoosh:memory", include_str!("../lua/memory.lua")),
    ("kawoosh:dir", include_str!("../lua/dir.lua")),
    // After dir: it deletes through `kawoosh.dir.remove`.
    ("kawoosh:du", include_str!("../lua/du.lua")),
    // After dir: a directory is the file manager's before a file is
    // asked what it is. Before hex: a picture is the picture pane's
    // before it is bytes.
    ("kawoosh:image", include_str!("../lua/image.lua")),
    ("kawoosh:hex", include_str!("../lua/hex.lua")),
    ("kawoosh:picker", include_str!("../lua/picker.lua")),
    ("kawoosh:search", include_str!("../lua/search.lua")),
    ("kawoosh:lists", include_str!("../lua/lists.lua")),
    ("kawoosh:dirs", include_str!("../lua/dirs.lua")),
    ("kawoosh:tools", include_str!("../lua/tools.lua")),
    ("kawoosh:launcher", include_str!("../lua/launcher.lua")),
    ("kawoosh:themes", include_str!("../lua/themes.lua")),
    // After themes: its cards set the sample in the theme on show.
    ("kawoosh:fonts", include_str!("../lua/fonts.lua")),
    // After themes: the lab shares its sample.
    ("kawoosh:theme_lab", include_str!("../lua/theme_lab.lua")),
    ("kawoosh:grammars", include_str!("../lua/grammars.lua")),
    ("kawoosh:pairs", include_str!("../lua/pairs.lua")),
    // After picker: `:node actions` opens one.
    (
        "kawoosh:node_actions",
        include_str!("../lua/node_actions.lua"),
    ),
    ("kawoosh:secrets", include_str!("../lua/secrets.lua")),
    // After pairs: its keys are newer, and pass to pairs' where a buffer
    // is not timed.
    ("kawoosh:timed", include_str!("../lua/timed.lua")),
    // After lists: its diagnostics segment opens `:diagnostics`.
    ("kawoosh:status", include_str!("../lua/status.lua")),
    // After picker: its pickers are sources.
    ("kawoosh:vcs", include_str!("../lua/vcs.lua")),
    ("kawoosh:qd", include_str!("../lua/qd.lua")),
    // Last: its sections name every plugin's settings.
    ("kawoosh:settings", include_str!("../lua/settings.lua")),
];
