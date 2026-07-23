use gpui::KeyBinding;

use crate::actions::*;

#[cfg(not(target_os = "macos"))]
pub fn default_keybinds() -> [KeyBinding; 15] {
    [
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("escape", Quit, None),
        KeyBinding::new("ctrl-1", TabSelect { idx: 0 }, None),
        KeyBinding::new("ctrl-2", TabSelect { idx: 1 }, None),
        KeyBinding::new("ctrl-3", TabSelect { idx: 2 }, None),
        KeyBinding::new("ctrl-4", TabSelect { idx: 3 }, None),
        KeyBinding::new("ctrl-5", TabSelect { idx: 4 }, None),
        KeyBinding::new("ctrl-6", TabSelect { idx: 5 }, None),
        KeyBinding::new("ctrl-7", TabSelect { idx: 6 }, None),
        KeyBinding::new("ctrl-8", TabSelect { idx: 7 }, None),
        KeyBinding::new("ctrl-9", TabSelect { idx: 8 }, None),
        KeyBinding::new("ctrl-n", TabNext, None),
        KeyBinding::new("ctrl-p", TabPrev, None),
        KeyBinding::new("ctrl-t", TabNew, None),
        KeyBinding::new("ctrl-w", TabClose { idx: None }, None),
    ]
}

#[cfg(target_os = "macos")]
pub fn default_keybinds() -> [KeyBinding; 15] {
    [
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("escape", Quit, None),
        KeyBinding::new("cmd-1", TabSelect { idx: 0 }, None),
        KeyBinding::new("cmd-2", TabSelect { idx: 1 }, None),
        KeyBinding::new("cmd-3", TabSelect { idx: 2 }, None),
        KeyBinding::new("cmd-4", TabSelect { idx: 3 }, None),
        KeyBinding::new("cmd-5", TabSelect { idx: 4 }, None),
        KeyBinding::new("cmd-6", TabSelect { idx: 5 }, None),
        KeyBinding::new("cmd-7", TabSelect { idx: 6 }, None),
        KeyBinding::new("cmd-8", TabSelect { idx: 7 }, None),
        KeyBinding::new("cmd-9", TabSelect { idx: 8 }, None),
        KeyBinding::new("cmd-n", TabNext, None),
        KeyBinding::new("cmd-p", TabPrev, None),
        KeyBinding::new("cmd-t", TabNew, None),
        KeyBinding::new("cmd-w", TabClose { idx: None }, None),
    ]
}
